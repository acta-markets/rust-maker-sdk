use solana_commitment_config::CommitmentConfig;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    account::Account, instruction::Instruction, message::AddressLookupTableAccount, pubkey::Pubkey,
    signature::Signature, signer::Signer, transaction::Transaction,
};

use crate::chain::ChainError;
use crate::chain::accounts::{GlobalConfigInfo, MarketInfo, PositionInfo};
use crate::chain::ix::{
    DepositPremiumIxArgs, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID, WithdrawPremiumIxArgs,
    build_deposit_premium_ixs, build_withdraw_premium_ixs,
};
use crate::chain::settlement::required_settlement;
use crate::chain::wsol::{
    NativeSolBudget, NativeSolFunding, WrapPlan, WsolWrapRequest, is_native_mint, plan_wsol_wrap,
    token_account_rent_lamports,
};

const ADDRESS_LOOKUP_TABLE_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("AddressLookupTab1e1111111111111111111111111");
const LOOKUP_TABLE_META_LEN: usize = 56;

fn parse_lookup_table(
    key: Pubkey,
    account: &Account,
) -> Result<AddressLookupTableAccount, ChainError> {
    if account.owner != ADDRESS_LOOKUP_TABLE_PROGRAM_ID {
        return Err(ChainError::AccountOwnerMismatch {
            expected: ADDRESS_LOOKUP_TABLE_PROGRAM_ID,
            actual: account.owner,
        });
    }
    let data = &account.data;
    let (Some(kind), Some(deactivation), Some(addresses)) = (
        data.get(..4),
        data.get(4..12),
        data.get(LOOKUP_TABLE_META_LEN..),
    ) else {
        return Err(ChainError::InvalidAccountData);
    };
    if kind != 1u32.to_le_bytes() || addresses.len() % 32 != 0 {
        return Err(ChainError::InvalidAccountData);
    }
    if deactivation != u64::MAX.to_le_bytes() {
        return Err(ChainError::LookupTableDeactivated(key));
    }
    Ok(AddressLookupTableAccount {
        key,
        addresses: addresses
            .as_chunks::<32>()
            .0
            .iter()
            .map(|bytes| Pubkey::new_from_array(*bytes))
            .collect(),
    })
}

pub struct ChainClient {
    rpc: RpcClient,
    program_id: Pubkey,
    commitment: CommitmentConfig,
    #[cfg(feature = "ws-client")]
    confirmation_ws_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SendOptions {
    pub compute_unit_limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DepositPremiumArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Option<Pubkey>,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct WithdrawPremiumArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Option<Pubkey>,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct FundPositionArgs {
    pub maker_owner: Pubkey,
    pub position_pda: Pubkey,
    pub create_atas: bool,
    pub native_sol: NativeSolFunding,
}

struct PreparedFundPosition {
    instructions: Vec<Instruction>,
    native_budget: Option<NativeSolBudget>,
}

impl ChainClient {
    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }

    pub fn from_rpc(rpc: RpcClient, program_id: Pubkey) -> Self {
        let commitment = rpc.commitment();
        Self {
            rpc,
            program_id,
            commitment,
            #[cfg(feature = "ws-client")]
            confirmation_ws_url: None,
        }
    }

    /// Use Solana signature subscriptions for confirming conveniences instead of
    /// the legacy RPC confirmation fallback. Batch submission remains submit-only.
    #[cfg(feature = "ws-client")]
    pub fn with_confirmation_ws_url(mut self, url: impl Into<String>) -> Self {
        self.confirmation_ws_url = Some(url.into());
        self
    }

    pub fn program_id(&self) -> Pubkey {
        self.program_id
    }

    pub fn new(rpc_url: &str, program_id: Pubkey, commitment: CommitmentConfig) -> Self {
        let rpc = RpcClient::new_with_commitment(rpc_url.to_string(), commitment);
        Self {
            rpc,
            program_id,
            commitment,
            #[cfg(feature = "ws-client")]
            confirmation_ws_url: None,
        }
    }

    /// Submit and wait for confirmation at the client commitment; execution failure is an error.
    pub async fn deposit_premium(
        &self,
        args: DepositPremiumArgs,
        maker_owner: &(dyn Signer + Sync),
        fee_payer: Option<&(dyn Signer + Sync)>,
    ) -> Result<Signature, ChainError> {
        let payer = fee_payer.map_or(maker_owner, |payer| payer);
        let ixs = self
            .build_deposit_premium_ixs(&args, payer.pubkey())
            .await?;
        self.send_instructions_with_options(ixs, payer, &[maker_owner], SendOptions::default())
            .await
    }

    /// Submit and wait for confirmation at the client commitment; execution failure is an error.
    pub async fn withdraw_premium(
        &self,
        args: WithdrawPremiumArgs,
        maker_owner: &(dyn Signer + Sync),
        fee_payer: Option<&(dyn Signer + Sync)>,
    ) -> Result<Signature, ChainError> {
        let payer = fee_payer.map_or(maker_owner, |payer| payer);
        let ixs = self
            .build_withdraw_premium_ixs(&args, payer.pubkey())
            .await?;
        self.send_instructions_with_options(ixs, payer, &[maker_owner], SendOptions::default())
            .await
    }

    /// Submit and wait for confirmation at the client commitment; execution failure is an error.
    pub async fn fund_position(
        &self,
        args: FundPositionArgs,
        maker_owner: &(dyn Signer + Sync),
        fee_payer: Option<&(dyn Signer + Sync)>,
    ) -> Result<Signature, ChainError> {
        let payer = fee_payer.map_or(maker_owner, |payer| payer);
        let prepared = self.prepare_fund_position(&args, payer.pubkey()).await?;
        let (signature, blockhash) = self
            .submit_instructions(
                prepared.instructions,
                payer,
                &[maker_owner],
                SendOptions::default(),
                prepared.native_budget,
                None,
            )
            .await?;
        self.confirm_submitted(signature, blockhash).await
    }

    pub async fn build_deposit_premium_ixs(
        &self,
        args: &DepositPremiumArgs,
        fee_payer: Pubkey,
    ) -> Result<Vec<Instruction>, ChainError> {
        let token_program = self
            .resolve_token_program(&args.premium_mint, args.token_program)
            .await?;
        build_deposit_premium_ixs(
            &self.program_id,
            &DepositPremiumIxArgs {
                maker_owner: args.maker_owner,
                amount: args.amount,
                premium_mint: args.premium_mint,
                token_program,
                create_atas: args.create_atas,
            },
            fee_payer,
        )
        .map_err(ChainError::from)
    }

    pub async fn build_withdraw_premium_ixs(
        &self,
        args: &WithdrawPremiumArgs,
        fee_payer: Pubkey,
    ) -> Result<Vec<Instruction>, ChainError> {
        let token_program = self
            .resolve_token_program(&args.premium_mint, args.token_program)
            .await?;
        build_withdraw_premium_ixs(
            &self.program_id,
            &WithdrawPremiumIxArgs {
                maker_owner: args.maker_owner,
                amount: args.amount,
                premium_mint: args.premium_mint,
                token_program,
                create_atas: args.create_atas,
            },
            fee_payer,
        )
        .map_err(ChainError::from)
    }

    pub async fn build_fund_position_ixs(
        &self,
        args: &FundPositionArgs,
        fee_payer: Pubkey,
    ) -> Result<Vec<Instruction>, ChainError> {
        Ok(self
            .prepare_fund_position(args, fee_payer)
            .await?
            .instructions)
    }

    async fn prepare_fund_position(
        &self,
        args: &FundPositionArgs,
        fee_payer: Pubkey,
    ) -> Result<PreparedFundPosition, ChainError> {
        let position_data = self.fetch_account(&args.position_pda).await?;
        let pos = PositionInfo::from_account(&position_data, &self.program_id)?;
        let market_data = self.fetch_account(&pos.market_pda).await?;
        let market = MarketInfo::from_account(&market_data, &self.program_id)?;

        let snapshot = super::batch::PositionSnapshot {
            address: args.position_pda,
            market_address: pos.market_pda,
            position: pos.clone(),
            market: market.clone(),
        };
        let mut fund_ixs = snapshot.fund_instructions(
            &self.program_id,
            args.maker_owner,
            fee_payer,
            args.create_atas,
        )?;

        let NativeSolFunding::WrapIfNeeded { reserve_lamports } = args.native_sol else {
            return Ok(PreparedFundPosition {
                instructions: fund_ixs,
                native_budget: None,
            });
        };
        let (settlement_mint, settlement_program) = match pos.terms.position_type() {
            crate::types::PositionType::CoveredCall => {
                (market.quote_mint, market.quote_token_program_id)
            }
            crate::types::PositionType::CashSecuredPut => {
                (market.underlying_mint, market.underlying_token_program_id)
            }
            _ => return Err(ChainError::InvalidSettlementState),
        };
        if !is_native_mint(&settlement_mint) {
            return Ok(PreparedFundPosition {
                instructions: fund_ixs,
                native_budget: None,
            });
        }
        let expected_settlement = required_settlement(
            pos.terms.to_chain().position_type().into(),
            pos.terms.strike().value(),
            pos.quantity.value(),
            market.quote_decimals.value(),
            market.underlying_decimals.value(),
        )?;
        if expected_settlement == 0 {
            return Err(ChainError::InvalidAccountData);
        }
        if settlement_program != crate::chain::wsol::TOKEN_PROGRAM_ID {
            return Err(ChainError::InvalidNativeTokenProgram(settlement_program));
        }
        let funding_ata = crate::chain::ix::derive_associated_token_address(
            &args.maker_owner,
            &settlement_mint,
            &settlement_program,
        );
        let position_funding_ata = crate::chain::ix::derive_associated_token_address(
            &args.position_pda,
            &settlement_mint,
            &settlement_program,
        );
        let (funding_account, additional_ata_rent_lamports) =
            if args.create_atas && fee_payer == args.maker_owner {
                let [position_account, funding_account]: [Option<Account>; 2] = self
                    .rpc
                    .get_multiple_accounts_with_commitment(
                        &[position_funding_ata, funding_ata],
                        self.commitment,
                    )
                    .await?
                    .value
                    .try_into()
                    .map_err(|_| ChainError::InvalidAccountData)?;
                let rent = if position_account.is_none() {
                    token_account_rent_lamports(&self.rpc).await?
                } else {
                    0
                };
                (funding_account, rent)
            } else {
                (
                    self.rpc
                        .get_account_with_commitment(&funding_ata, self.commitment)
                        .await?
                        .value,
                    0,
                )
            };
        match plan_wsol_wrap(
            &self.rpc,
            funding_account,
            WsolWrapRequest {
                owner: args.maker_owner,
                funding_ata,
                token_program: settlement_program,
                expected_settlement,
                reserve_lamports,
                allow_account_creation: args.create_atas,
                fee_payer,
                additional_ata_rent_lamports,
            },
        )
        .await?
        {
            WrapPlan::NotNeeded => Ok(PreparedFundPosition {
                instructions: fund_ixs,
                native_budget: None,
            }),
            WrapPlan::Wrap {
                instructions: wrap_ixs,
                budget,
            } => {
                let deposit_ix = fund_ixs.pop().ok_or(ChainError::InvalidAccountData)?;
                fund_ixs.extend(wrap_ixs);
                fund_ixs.push(deposit_ix);
                Ok(PreparedFundPosition {
                    instructions: fund_ixs,
                    native_budget: Some(budget),
                })
            }
        }
    }

    pub async fn fetch_position_info(
        &self,
        position_pda: &Pubkey,
    ) -> Result<PositionInfo, ChainError> {
        let data = self.fetch_account(position_pda).await?;
        PositionInfo::from_account(&data, &self.program_id)
    }

    pub async fn fetch_market_info(&self, market_pda: &Pubkey) -> Result<MarketInfo, ChainError> {
        let market_data = self.fetch_account(market_pda).await?;
        MarketInfo::from_account(&market_data, &self.program_id)
    }

    pub async fn fetch_market_quote_mint(&self, market_pda: &Pubkey) -> Result<Pubkey, ChainError> {
        Ok(self.fetch_market_info(market_pda).await?.quote_mint)
    }

    /// Reads the canonical GlobalConfig PDA and returns its checked hot authority.
    ///
    /// Vault opens use this key as both transaction fee payer and account 0, so
    /// accepting a venue-provided value would let the venue choose the signer.
    pub async fn hot_authority(&self) -> Result<Pubkey, ChainError> {
        let (global_config, bump) =
            Pubkey::find_program_address(&[b"global_config"], &self.program_id);
        let account = self.fetch_account(&global_config).await?;
        let config = GlobalConfigInfo::from_account(&account, &self.program_id)?;
        if config.bump != bump {
            return Err(ChainError::Identity("global config PDA"));
        }
        Ok(config.hot_authority)
    }

    /// Fetch the raw token balance (u64 atoms) of a token account.
    pub async fn fetch_token_balance(&self, token_account: &Pubkey) -> Result<u64, ChainError> {
        let result = self.rpc.get_token_account_balance(token_account).await?;
        result
            .amount
            .parse::<u64>()
            .map_err(|_| ChainError::InvalidAccountData)
    }

    /// Active lookup tables by key, for verifying served v0 transactions.
    pub async fn address_lookup_tables(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<AddressLookupTableAccount>, ChainError> {
        let accounts = self
            .rpc
            .get_multiple_accounts_with_commitment(keys, self.commitment)
            .await?
            .value;
        keys.iter()
            .zip(accounts)
            .map(|(key, account)| match account {
                Some(account) => parse_lookup_table(*key, &account),
                None => Err(ChainError::MissingLookupTable(*key)),
            })
            .collect()
    }

    async fn resolve_token_program(
        &self,
        mint: &Pubkey,
        token_program: Option<Pubkey>,
    ) -> Result<Pubkey, ChainError> {
        if let Some(program) = token_program {
            return ensure_supported_token_program(program);
        }
        let account = self.rpc.get_account(mint).await?;
        ensure_supported_token_program(account.owner)
    }

    pub(crate) async fn fetch_account(&self, pubkey: &Pubkey) -> Result<Account, ChainError> {
        self.rpc.get_account(pubkey).await.map_err(ChainError::from)
    }

    /// Program accounts matching `filters`, fetched at the client's commitment.
    pub(crate) async fn program_accounts(
        &self,
        filters: Vec<solana_rpc_client::api::filter::RpcFilterType>,
    ) -> Result<Vec<(Pubkey, Account)>, ChainError> {
        use solana_rpc_client::api::config::{
            RpcAccountInfoConfig, RpcProgramAccountsConfig, UiAccountEncoding,
        };
        self.rpc
            .get_program_ui_accounts_with_config(
                &self.program_id,
                RpcProgramAccountsConfig {
                    filters: Some(filters),
                    account_config: RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        commitment: Some(self.commitment),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await?
            .into_iter()
            .map(|(address, account)| {
                account
                    .decode::<Account>()
                    .map(|account| (address, account))
                    .ok_or(ChainError::InvalidAccountData)
            })
            .collect()
    }

    pub(crate) async fn fetch_optional_account(
        &self,
        pubkey: &Pubkey,
    ) -> Result<Option<Account>, ChainError> {
        Ok(self
            .rpc
            .get_account_with_commitment(pubkey, self.commitment)
            .await?
            .value)
    }

    /// Submit and wait for confirmation at the client commitment; execution failure is an error.
    pub async fn send_instructions_with_options(
        &self,
        instructions: Vec<Instruction>,
        fee_payer: &(dyn Signer + Sync),
        extra_signers: &[&(dyn Signer + Sync)],
        options: SendOptions,
    ) -> Result<Signature, ChainError> {
        let (signature, blockhash) = self
            .submit_instructions(instructions, fee_payer, extra_signers, options, None, None)
            .await?;
        self.confirm_submitted(signature, blockhash).await
    }

    /// Submit without confirmation using a blockhash shared across the caller's batch.
    pub async fn send_instructions_with_blockhash(
        &self,
        instructions: Vec<Instruction>,
        fee_payer: &(dyn Signer + Sync),
        extra_signers: &[&(dyn Signer + Sync)],
        options: SendOptions,
        blockhash: solana_sdk::hash::Hash,
    ) -> Result<Signature, ChainError> {
        self.submit_instructions(
            instructions,
            fee_payer,
            extra_signers,
            options,
            None,
            Some(blockhash),
        )
        .await
        .map(|(signature, _)| signature)
    }

    async fn submit_instructions(
        &self,
        mut instructions: Vec<Instruction>,
        fee_payer: &(dyn Signer + Sync),
        extra_signers: &[&(dyn Signer + Sync)],
        options: SendOptions,
        native_budget: Option<NativeSolBudget>,
        blockhash: Option<solana_sdk::hash::Hash>,
    ) -> Result<(Signature, solana_sdk::hash::Hash), ChainError> {
        if let Some(limit) = options.compute_unit_limit {
            instructions.insert(
                0,
                solana_compute_budget_interface::ComputeBudgetInstruction::set_compute_unit_limit(
                    limit,
                ),
            );
        }
        let recent = match blockhash {
            Some(hash) => hash,
            None => self.rpc.get_latest_blockhash().await?,
        };

        let mut tx = Transaction::new_with_payer(&instructions, Some(&fee_payer.pubkey()));
        tx.message.recent_blockhash = recent;
        if let Some(budget) = native_budget {
            let fee_lamports = if fee_payer.pubkey() == budget.owner {
                self.rpc.get_fee_for_message(&tx.message).await?
            } else {
                0
            };
            let available_lamports = self.rpc.get_balance(&budget.owner).await?;
            budget.validate(available_lamports, fee_lamports)?;
        }
        let mut signers: Vec<&(dyn Signer + Sync)> = Vec::with_capacity(1 + extra_signers.len());
        signers.push(fee_payer);
        for signer in extra_signers {
            if signer.pubkey() != fee_payer.pubkey() {
                signers.push(*signer);
            }
        }
        sign_transaction(&mut tx, &signers, recent)?;
        let signature = self.rpc.send_transaction(&tx).await?;
        Ok((signature, recent))
    }
    async fn confirm_submitted(
        &self,
        signature: Signature,
        blockhash: solana_sdk::hash::Hash,
    ) -> Result<Signature, ChainError> {
        #[cfg(feature = "ws-client")]
        if let Some(url) = &self.confirmation_ws_url {
            let statuses = self
                .confirm_signatures(url, &[signature], std::time::Duration::from_secs(60))
                .await?;
            return match statuses.as_slice() {
                [super::ConfirmationStatus::Confirmed] => Ok(signature),
                [super::ConfirmationStatus::Failed(error)] => Err(ChainError::Confirmation(
                    format!("transaction {signature} failed: {error}"),
                )),
                _ => Err(ChainError::Confirmation(format!(
                    "transaction {signature} confirmation unresolved"
                ))),
            };
        }
        self.rpc
            .confirm_transaction_with_spinner(&signature, &blockhash, self.commitment)
            .await?;
        Ok(signature)
    }
}

fn sign_transaction(
    transaction: &mut Transaction,
    signers: &[&(dyn Signer + Sync)],
    recent_blockhash: solana_sdk::hash::Hash,
) -> Result<(), ChainError> {
    transaction.try_sign(signers, recent_blockhash)?;
    Ok(())
}

fn ensure_supported_token_program(program: Pubkey) -> Result<Pubkey, ChainError> {
    if program == TOKEN_PROGRAM_ID || program == TOKEN_2022_PROGRAM_ID {
        Ok(program)
    } else {
        Err(ChainError::InvalidAccountData)
    }
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rpc_mock.rs"]
mod mock;
