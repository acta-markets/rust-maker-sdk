//! Crank a vault's capital cycle from the governance or delegate key: one step per pass.

use std::str::FromStr;
use std::time::{Duration, Instant};

use acta_maker_sdk::ACTA_PROGRAM_ID;
use acta_maker_sdk::chain::vault::{
    CapitalStep, VAULT_CAPITAL_BATCH_USERS, VaultAccount, VaultManager, VaultRequest,
    VaultRequestKind, VaultSafety, VaultView, WindowPolicy, WindowTracker, next_step,
    oracle_source_pyth_feed, plan_withdrawals, pyth_price_feed_account,
    vault_capital_batch_compute_units,
};
use acta_maker_sdk::chain::{ChainClient, SendOptions};
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client::api::config::{
    RpcAccountInfoConfig, RpcProgramAccountsConfig, UiAccountEncoding,
};
use solana_rpc_client::api::filter::{Memcmp, RpcFilterType};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer, read_keypair_file};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const POLICY: WindowPolicy = WindowPolicy {
    reopen: true,
    deposit_window: Duration::from_secs(120),
    trading_window: Duration::from_secs(600),
};

struct Crank {
    rpc: RpcClient,
    chain: ChainClient,
    operator: Keypair,
    program_id: Pubkey,
    vault_pda: Pubkey,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let path = std::env::var("ACTA_KEYPAIR_PATH")
        .map_err(|_| "set ACTA_KEYPAIR_PATH to the vault governance or delegate keypair")?;
    let vault_pda = std::env::var("ACTA_VAULT_PDA").map_err(|_| "set ACTA_VAULT_PDA")?;
    let rpc_url = match std::env::var("ACTA_RPC_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => "https://api.devnet.solana.com".into(),
        Err(error) => return Err(error.into()),
    };
    let program_id = Pubkey::new_from_array(ACTA_PROGRAM_ID);
    let commitment = CommitmentConfig::confirmed();
    let crank = Crank {
        rpc: RpcClient::new_with_commitment(rpc_url.clone(), commitment),
        chain: ChainClient::new(&rpc_url, program_id, commitment),
        operator: read_keypair_file(path)?,
        program_id,
        vault_pda: Pubkey::from_str(&vault_pda)?,
    };

    let mut window = WindowTracker::default();
    loop {
        match crank.pass(&mut window).await {
            Ok(Some(step)) => tracing::info!(?step, "capital step sent"),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "capital pass failed"),
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

impl Crank {
    async fn pass(&self, window: &mut WindowTracker) -> Result<Option<CapitalStep>> {
        let vault = VaultAccount::parse(&self.rpc.get_account_data(&self.vault_pda).await?)?;
        let clock = window.observe(vault.accepting_deposits, Instant::now());
        let Some(step) = next_step(VaultView::from(&vault), clock, POLICY) else {
            return Ok(None);
        };
        let manager = VaultManager::from_vault(self.program_id, self.operator.pubkey(), &vault)?;
        match step {
            CapitalStep::CloseDeposits => {
                if vault.pending_deposit_count > 0 || vault.pending_withdraw_count > 0 {
                    self.price_update(manager.addresses.main_oracle_source)
                        .await?;
                    self.price_update(manager.addresses.second_oracle_source)
                        .await?;
                }
                self.send(manager.set_accepting_deposits(false), None)
                    .await?
            }
            CapitalStep::OpenDeposits => {
                self.send(manager.set_accepting_deposits(true), None)
                    .await?
            }
            CapitalStep::ComputeEquity => {
                let main = self
                    .price_update(manager.addresses.main_oracle_source)
                    .await?;
                let second = self
                    .price_update(manager.addresses.second_oracle_source)
                    .await?;
                self.send(manager.process_equity(main, second), None)
                    .await?;
            }
            CapitalStep::ProcessDeposits => {
                let users: Vec<Pubkey> = self
                    .requests(VaultRequestKind::Deposit)
                    .await?
                    .into_iter()
                    .map(|request| request.user)
                    .collect();
                for batch in users.chunks(VAULT_CAPITAL_BATCH_USERS) {
                    let units = vault_capital_batch_compute_units(manager.deposit_batch_shape(
                        batch,
                        batch.len(),
                        batch.len(),
                    ));
                    self.send(manager.process_deposits(batch)?, Some(units))
                        .await?;
                }
            }
            CapitalStep::ProcessWithdrawals => {
                let queued: Vec<(Pubkey, u64)> = self
                    .requests(VaultRequestKind::Withdraw)
                    .await?
                    .into_iter()
                    .map(|request| (request.user, request.amount))
                    .collect();
                let supply = self
                    .rpc
                    .get_token_supply(&vault.share_mint)
                    .await?
                    .amount
                    .parse()?;
                let plan = plan_withdrawals(&queued, supply);
                for batch in plan.partial.chunks(VAULT_CAPITAL_BATCH_USERS) {
                    let units = vault_capital_batch_compute_units(manager.withdrawal_batch_shape(
                        batch,
                        batch.len(),
                        batch.len(),
                    ));
                    self.send(manager.process_withdrawals(batch)?, Some(units))
                        .await?;
                }
                if let Some(user) = plan.final_user
                    && (vault.pending_deposit_count == 0
                        || vault.safety == VaultSafety::WithdrawOnly)
                {
                    self.send(manager.process_final_withdrawal(user), None)
                        .await?;
                }
            }
            CapitalStep::FinalizeCycle => self.send(manager.finalize_cycle(), None).await?,
        }
        Ok(Some(step))
    }

    async fn price_update(&self, oracle_source: Pubkey) -> Result<Pubkey> {
        let feed = oracle_source_pyth_feed(&self.rpc.get_account_data(&oracle_source).await?)?;
        Ok(pyth_price_feed_account(&feed, 0))
    }

    async fn requests(&self, kind: VaultRequestKind) -> Result<Vec<VaultRequest>> {
        let config = RpcProgramAccountsConfig {
            filters: Some(vec![
                RpcFilterType::DataSize(VaultRequest::LEN as u64),
                RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, vec![kind.discriminator()])),
                RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                    VaultRequest::VAULT_OFFSET,
                    self.vault_pda.to_bytes().to_vec(),
                )),
            ]),
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                ..RpcAccountInfoConfig::default()
            },
            ..RpcProgramAccountsConfig::default()
        };
        let accounts = self
            .rpc
            .get_program_ui_accounts_with_config(&self.program_id, config)
            .await?;
        let mut requests = Vec::with_capacity(accounts.len());
        for (_, account) in accounts {
            let data = account.data.decode().ok_or("undecodable request account")?;
            requests.push(VaultRequest::parse(&data, kind)?);
        }
        Ok(requests)
    }

    async fn send(&self, ix: Instruction, compute_unit_limit: Option<u32>) -> Result<()> {
        let signature = self
            .chain
            .send_instructions_with_options(
                vec![ix],
                &self.operator,
                &[],
                SendOptions { compute_unit_limit },
            )
            .await?;
        tracing::info!(%signature, "sent");
        Ok(())
    }
}
