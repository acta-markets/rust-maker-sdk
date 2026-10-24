//! RPC and Jupiter edges for curator plans.
use super::{
    VaultChainError,
    router::{Intent, Plan, TxParts},
    venues::jupiter::{BuildResponse, Quote, Request},
};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    account::Account, pubkey::Pubkey, signature::Signature, signer::Signer,
    transaction::VersionedTransaction,
};
use std::{collections::BTreeMap, time::Duration};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct JupiterClient {
    http: reqwest::Client,
    endpoint: String,
}
impl JupiterClient {
    pub fn new(api_key: &str) -> Result<Self, Error> {
        Self::with_endpoint("https://api.jup.ag/swap/v2/build", api_key)
    }
    pub fn from_env() -> Result<Self, Error> {
        Self::new(&std::env::var("JUPITER_API_KEY")?)
    }
    pub fn with_endpoint(endpoint: &str, api_key: &str) -> Result<Self, Error> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-api-key", api_key.parse()?);
        Ok(Self {
            http: reqwest::Client::builder()
                .default_headers(headers)
                .timeout(Duration::from_secs(20))
                .build()?,
            endpoint: endpoint.into(),
        })
    }
    pub async fn quote(&self, request: Request, max_accounts: u8) -> Result<Quote, Error> {
        let params = [
            ("inputMint", request.input_mint.to_string()),
            ("outputMint", request.output_mint.to_string()),
            ("amount", request.amount.to_string()),
            ("taker", request.owner.to_string()),
            ("payer", request.payer.to_string()),
            ("destinationTokenAccount", request.destination.to_string()),
            ("slippageBps", request.slippage_bps.to_string()),
            ("maxAccounts", max_accounts.to_string()),
            ("wrapAndUnwrapSol", "false".into()),
        ];
        let response = self
            .http
            .get(&self.endpoint)
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .json::<BuildResponse>()
            .await?;
        Ok(response.into_quote(request)?)
    }
}

/// One getMultipleAccounts for a configured dependency manifest, at confirmed commitment.
pub async fn fetch_snapshot(
    rpc: &RpcClient,
    keys: &[Pubkey],
) -> Result<BTreeMap<Pubkey, Account>, Error> {
    let response = rpc
        .get_multiple_accounts_with_commitment(
            keys,
            solana_commitment_config::CommitmentConfig::confirmed(),
        )
        .await?;
    Ok(keys
        .iter()
        .copied()
        .zip(response.value)
        .filter_map(|(key, account)| account.map(|a| (key, a)))
        .collect())
}

fn verify_quote_tables(
    quote: &mut Quote,
    snapshot: &BTreeMap<Pubkey, Account>,
) -> Result<(), Error> {
    for supplied in &mut quote.tables {
        let on_chain = table(snapshot, supplied.key)?;
        if supplied.addresses != on_chain.addresses {
            return Err("Jupiter lookup table contents differ from RPC".into());
        }
        *supplied = on_chain;
    }
    Ok(())
}

/// Initial request plus at most three smaller requests when planning does not fit.
pub async fn fit_with_quotes<F, Fut, P>(mut fetch: F, mut plan: P) -> Result<Plan, Error>
where
    F: FnMut(u8) -> Fut,
    Fut: std::future::Future<Output = Result<Quote, Error>>,
    P: FnMut(&Quote) -> Result<Plan, VaultChainError>,
{
    for budget in [48, 36, 24, 12] {
        let quote = fetch(budget).await?;
        match plan(&quote) {
            Ok(plan) => return Ok(plan),
            Err(VaultChainError::InvalidInput("curator transaction does not fit"))
                if budget != 12 =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(VaultChainError::InvalidInput("curator transaction does not fit").into())
}

/// IO boundary: refresh must re-read the slot and re-quote for its actual amount.
pub trait Driver {
    fn refresh(&mut self, intent: Intent)
    -> impl std::future::Future<Output = Result<Plan, Error>>;
    fn send_confirmed(
        &mut self,
        parts: &TxParts,
    ) -> impl std::future::Future<Output = Result<Signature, Error>>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Executed {
        signatures: Vec<Signature>,
    },
    /// Finished an existing slot; the requested intent was not executed.
    Recovered {
        signature: Signature,
    },
}
#[derive(Debug, thiserror::Error)]
#[error("stage {signature} confirmed, completion failed: {source}")]
pub struct StageError {
    pub signature: Signature,
    #[source]
    pub source: Error,
}

pub async fn execute(driver: &mut impl Driver, intent: Intent) -> Result<Outcome, Error> {
    use super::VaultSwapKind::Staging;
    use crate::types::VaultStagingStep::Unstage;
    let plan = driver.refresh(intent).await?;
    let signature = driver.send_confirmed(plan.first()).await?;
    if let Plan::Single(parts) = plan {
        return Ok(if matches!(parts.kind, Staging { step: Unstage, .. }) {
            Outcome::Recovered { signature }
        } else {
            Outcome::Executed {
                signatures: vec![signature],
            }
        });
    }
    let Staging {
        asset,
        generation,
        leg: staged_leg,
        ..
    } = plan.first().kind
    else {
        return Err(Box::new(StageError {
            signature,
            source: "expected staging transaction".into(),
        }));
    };
    let expected = (asset, generation);
    let (completion, rolled_back) = async {
        let next = driver.refresh(intent).await?;
        let Plan::Single(parts) = next else {
            return Err("confirmed stage did not produce an open slot".into());
        };
        let Staging {
            step: Unstage,
            asset,
            generation,
            leg,
        } = parts.kind
        else {
            return Err("expected full-slot unstage after stage".into());
        };
        if (asset, generation) != expected {
            return Err("expected full-slot unstage after stage".into());
        }
        Ok((driver.send_confirmed(&parts).await?, leg == staged_leg))
    }
    .await
    .map_err(|source| Box::new(StageError { signature, source }) as Error)?;
    Ok(if rolled_back {
        Outcome::Recovered {
            signature: completion,
        }
    } else {
        Outcome::Executed {
            signatures: vec![signature, completion],
        }
    })
}

pub async fn sign(
    rpc: &RpcClient,
    parts: &TxParts,
    signer: &impl Signer,
) -> Result<VersionedTransaction, Error> {
    let message = parts.compile(signer.pubkey(), rpc.get_latest_blockhash().await?)?;
    Ok(VersionedTransaction::try_new(message, &[signer])?)
}
pub async fn send_confirmed(
    rpc: &RpcClient,
    parts: &TxParts,
    signer: &impl Signer,
) -> Result<Signature, Error> {
    let tx = sign(rpc, parts, signer).await?;
    rpc.send_and_confirm_transaction_with_spinner_and_commitment(
        &tx,
        solana_commitment_config::CommitmentConfig::confirmed(),
    )
    .await
    .map_err(|error| {
        format!(
            "submission/confirmation unresolved for {:?}: {error}",
            tx.signatures.first()
        )
        .into()
    })
}

const CLOCK: Pubkey = solana_sdk::pubkey!("SysvarC1ock11111111111111111111111111111111");
/// Discover once; every subsequent plan refresh fetches this dependency set in one batch.
pub struct Config {
    pub manager: super::VaultManager,
    pub market: Pubkey,
    pub sy: Pubkey,
    pub core: Pubkey,
    pub shard: u16,
    keys: Vec<Pubkey>,
    ata_setup: Vec<solana_sdk::instruction::Instruction>,
}
impl Config {
    #[allow(clippy::too_many_arguments)]
    pub async fn discover(
        rpc: &RpcClient,
        program: Pubkey,
        vault: Pubkey,
        actor: Pubkey,
        market: Pubkey,
        sy: Pubkey,
        core: Pubkey,
        shard: u16,
    ) -> Result<Self, Error> {
        use super::{
            PriceRoute, pda,
            venues::exponent::{CoreVault, GenericSy, Market},
        };
        let first = fetch_snapshot(rpc, &[vault, market, sy, core]).await?;
        let state = super::VaultAccount::from_account(account(&first, vault)?, &program)?;
        let manager = super::VaultManager::from_vault(program, actor, &state)?;
        if manager.addresses.vault != vault {
            return Err("vault PDA mismatch".into());
        }
        let m = Market::parse(
            market,
            &account_data(&first, market, super::venues::exponent::CLMM)?,
        )?;
        let s = GenericSy::parse(
            sy,
            &account_data(&first, sy, super::venues::exponent::GENERIC_SY)?,
        )?;
        let c = CoreVault::parse(
            core,
            &account_data(&first, core, super::venues::exponent::CORE)?,
        )?;
        let routes = [
            manager.addresses.main_price_route,
            manager.addresses.second_price_route,
            pda::price_route(&program, &s.base).0,
        ];
        let second = fetch_snapshot(rpc, &[routes.as_slice(), &[s.base]].concat()).await?;
        if m.pt != state.second_mint
            || m.sy != s.mint
            || m.vault != c.address
            || c.pt != m.pt
            || c.sy != s.mint
        {
            return Err("Exponent venue does not match vault pair".into());
        }
        let base_program = account(&second, s.base)?.owner;
        let mut keys = vec![
            vault,
            market,
            sy,
            core,
            CLOCK,
            manager.addresses.swap_venues,
            m.lookup_table,
            c.lookup_table,
            state.main_mint,
            state.second_mint,
            s.base,
        ];
        keys.extend(&s.interface_accounts);
        for (key, mint) in routes
            .into_iter()
            .zip([state.main_mint, state.second_mint, s.base])
        {
            let route = PriceRoute::from_account(account(&second, key)?, &program)?;
            if route.mint != mint {
                return Err("price route mint mismatch".into());
            }
            keys.extend(route.accounts(key, shard).iter().map(|a| a.pubkey));
        }
        let ata_setup: Vec<_> = [
            (s.base, base_program),
            (s.mint, crate::chain::ix::TOKEN_PROGRAM_ID),
            (c.yt, crate::chain::ix::TOKEN_PROGRAM_ID),
        ]
        .map(|(mint, program)| super::served::create_ata_idempotent(actor, vault, mint, program))
        .into();
        keys.extend(ata_setup.iter().map(|ix| ix.accounts[1].pubkey));
        keys.sort();
        keys.dedup();
        Ok(Self {
            manager,
            market,
            sy,
            core,
            shard,
            keys,
            ata_setup,
        })
    }
    pub fn snapshot_keys(&self) -> &[Pubkey] {
        &self.keys
    }
}
fn account(snapshot: &BTreeMap<Pubkey, Account>, key: Pubkey) -> Result<&Account, Error> {
    snapshot
        .get(&key)
        .ok_or_else(|| format!("snapshot dependency missing: {key}").into())
}
fn account_data(
    snapshot: &BTreeMap<Pubkey, Account>,
    key: Pubkey,
    owner: Pubkey,
) -> Result<Vec<u8>, Error> {
    let account = account(snapshot, key)?;
    if account.owner != owner {
        return Err("snapshot account owner mismatch".into());
    }
    Ok(account.data.clone())
}
fn table(
    snapshot: &BTreeMap<Pubkey, Account>,
    key: Pubkey,
) -> Result<solana_sdk::message::AddressLookupTableAccount, Error> {
    let data = account_data(
        snapshot,
        key,
        solana_sdk::pubkey!("AddressLookupTab1e1111111111111111111111111"),
    )?;
    if data.len() < 56
        || data[..4] != 1u32.to_le_bytes()
        || data[4..12] != u64::MAX.to_le_bytes()
        || (data.len() - 56) % 32 != 0
    {
        return Err("invalid or deactivated ALT".into());
    }
    let addresses = data[56..]
        .as_chunks::<32>()
        .0
        .iter()
        .map(|b| Ok(Pubkey::new_from_array(*b)))
        .collect::<Result<_, Error>>()?;
    Ok(solana_sdk::message::AddressLookupTableAccount { key, addresses })
}

pub struct Curator<'a, S> {
    rpc: &'a RpcClient,
    jupiter: &'a JupiterClient,
    signer: &'a S,
    config: Config,
    slippage_bps: u16,
    lifetime_secs: u64,
}
impl<'a, S: Signer> Curator<'a, S> {
    pub fn new(
        rpc: &'a RpcClient,
        jupiter: &'a JupiterClient,
        signer: &'a S,
        config: Config,
        slippage_bps: u16,
        lifetime_secs: u64,
    ) -> Result<Self, Error> {
        if signer.pubkey() != config.manager.args.actor || slippage_bps > 100 || lifetime_secs == 0
        {
            return Err("invalid curator signer, slippage or lifetime".into());
        }
        Ok(Self {
            rpc,
            jupiter,
            signer,
            config,
            slippage_bps,
            lifetime_secs,
        })
    }
    pub async fn plan(&self, intent: Intent) -> Result<Plan, Error> {
        use super::{
            PriceRoute, pda,
            router::{PriceClock, Router, evaluate},
            venues::exponent::{CoreVault, GenericSy, Market},
        };
        let cfg = &self.config;
        let snapshot = fetch_snapshot(self.rpc, &cfg.keys).await?;
        let state = super::VaultAccount::from_account(
            account(&snapshot, cfg.manager.addresses.vault)?,
            &cfg.manager.args.program_id,
        )?;
        let m = Market::parse(
            cfg.market,
            &account_data(&snapshot, cfg.market, super::venues::exponent::CLMM)?,
        )?;
        let sy = GenericSy::parse(
            cfg.sy,
            &account_data(&snapshot, cfg.sy, super::venues::exponent::GENERIC_SY)?,
        )?;
        let core = CoreVault::parse(
            cfg.core,
            &account_data(&snapshot, cfg.core, super::venues::exponent::CORE)?,
        )?;
        let clock = &account(&snapshot, CLOCK)?.data;
        let u64_at = |at| -> Result<u64, Error> {
            Ok(u64::from_le_bytes(
                clock.get(at..at + 8).ok_or("truncated clock")?.try_into()?,
            ))
        };
        let clock = PriceClock {
            now: u64_at(32)?,
            epoch: u64_at(16)?,
        };
        let routes = [
            cfg.manager.addresses.main_price_route,
            cfg.manager.addresses.second_price_route,
            pda::price_route(&cfg.manager.args.program_id, &sy.base).0,
        ]
        .map(|key| {
            PriceRoute::from_account(account(&snapshot, key)?, &cfg.manager.args.program_id)
                .map_err(Error::from)
        });
        let [main, second, staging] = routes;
        let (main, second, staging) = (main?, second?, staging?);
        let prices = [
            evaluate(&main, &snapshot, cfg.shard, clock)?,
            evaluate(&second, &snapshot, cfg.shard, clock)?,
            evaluate(&staging, &snapshot, cfg.shard, clock)?,
        ];
        let tables = vec![
            table(&snapshot, m.lookup_table)?,
            table(&snapshot, core.lookup_table)?,
        ];
        let registry = super::venues::SwapVenues::parse(&account_data(
            &snapshot,
            cfg.manager.addresses.swap_venues,
            cfg.manager.args.program_id,
        )?)?;
        let base_program = account(&snapshot, sy.base)?.owner;
        let base = crate::chain::ix::derive_associated_token_address(
            &cfg.manager.addresses.vault,
            &sy.base,
            &base_program,
        );
        let mut setup = vec![
            solana_compute_budget_interface::ComputeBudgetInstruction::set_compute_unit_limit(
                1_400_000,
            ),
        ];
        setup.extend(missing_ata_setup(&cfg.ata_setup, &snapshot));
        let pool_key = *sy.interface_accounts.first().ok_or("SY pool missing")?;
        let pool_account = account(&snapshot, pool_key)?;
        let pool = super::venues::exponent::StakePool::parse(
            pool_key,
            pool_account.owner,
            &pool_account.data,
        )?;
        if pool.mint != sy.base || pool.token_program != base_program {
            return Err("SY stake pool mint mismatch".into());
        }
        if pool.epoch < clock.epoch {
            setup.push(pool.update());
        }
        let router = Router {
            manager: &cfg.manager,
            account: &state,
            main: &main,
            second: &second,
            staging: &staging,
            registry: &registry,
            market: &m,
            sy: &sy,
            core: &core,
            tables: &tables,
            shard: cfg.shard,
            now: clock.now,
            valid_until: clock
                .now
                .checked_add(self.lifetime_secs)
                .ok_or("deadline overflow")?,
            base_program,
            setup,
            prices,
        };
        let buy = router.validate(intent)?;
        let amount = match state.staging.open {
            Some(slot) if !buy => slot.amount,
            _ if buy => intent.amount(),
            _ => sell_quote_amount(prices[1], intent.amount(), prices[2], self.slippage_bps)?,
        };
        let request = Request {
            owner: cfg.manager.addresses.vault,
            payer: self.signer.pubkey(),
            input_mint: if buy { state.main_mint } else { sy.base },
            output_mint: if buy { sy.base } else { state.main_mint },
            source: if buy {
                cfg.manager.addresses.vault_main_ata
            } else {
                base
            },
            destination: if buy {
                base
            } else {
                cfg.manager.addresses.vault_main_ata
            },
            amount,
            slippage_bps: self.slippage_bps,
        };
        let discounted = |value: u64| -> u64 {
            ((u128::from(value) * u128::from(10_000 - self.slippage_bps)) / 10_000) as u64
        };
        if let Some(slot) = state.staging.open
            && buy
        {
            return Ok(router.plan(
                intent,
                None,
                discounted(prices[2].convert(slot.amount, prices[1])?),
                0,
            )?);
        }
        fit_with_quotes(
            |budget| {
                let request = request.clone();
                async move {
                    let mut quote = self.jupiter.quote(request, budget).await?;
                    if !quote.tables.is_empty() {
                        let keys: Vec<_> = quote.tables.iter().map(|table| table.key).collect();
                        let snapshot = fetch_snapshot(self.rpc, &keys).await?;
                        verify_quote_tables(&mut quote, &snapshot)?;
                    }
                    Ok(quote)
                }
            },
            |quote| {
                let minimum_out = if buy {
                    discounted(prices[0].convert(intent.amount(), prices[1])?)
                } else {
                    quote.minimum
                };
                let staged = if buy {
                    quote.minimum
                } else {
                    quote.request.amount
                };
                router.plan(intent, Some(quote), minimum_out, staged)
            },
        )
        .await
    }
    /// Simulation only; staged plans simulate their first half, without submitting it.
    pub async fn simulate(&self, intent: Intent) -> Result<(), Error> {
        let plan = self.plan(intent).await?;
        let tx = sign(self.rpc, plan.first(), self.signer).await?;
        let result = self.rpc.simulate_transaction(&tx).await?;
        if let Some(error) = result.value.err {
            return Err(format!(
                "simulation failed: {error:?}; logs: {:?}",
                result.value.logs
            )
            .into());
        }
        Ok(())
    }
}
impl<S: Signer> Driver for Curator<'_, S> {
    async fn refresh(&mut self, intent: Intent) -> Result<Plan, Error> {
        self.plan(intent).await
    }
    async fn send_confirmed(&mut self, parts: &TxParts) -> Result<Signature, Error> {
        send_confirmed(self.rpc, parts, self.signer).await
    }
}

/// Backend submission retains the existing verifier, connection epoch and caller's send gate.
#[cfg(feature = "vault")]
#[allow(clippy::too_many_arguments)]
pub async fn backend_swap(
    client: &crate::ws::vault::VaultOperatorClient,
    rpc: &RpcClient,
    intent: Intent,
    manager: &super::VaultManager,
    state: super::VaultAccount,
    routes: super::PairPriceRoutes<'_>,
    minimum: u64,
    slippage_bps: u16,
    bounds: super::ComputeBudgetBounds,
    epoch: u64,
    gate: &impl crate::ws::vault::SendGate,
) -> Result<crate::ws::types::SwapSubmitResultData, Error> {
    let request = intent.backend_request(manager.addresses.vault, slippage_bps)?;
    let quote = client.swap_quote(request.clone()).await?;
    let keys = super::served_lookup_table_keys(&quote.tx_base64)?;
    let snapshot = fetch_snapshot(rpc, &keys).await?;
    let tables = keys
        .into_iter()
        .map(|key| table(&snapshot, key))
        .collect::<Result<Vec<_>, _>>()?;
    let intent = super::VaultSwapIntent {
        vault: manager,
        request: &request,
        quote: &quote,
        price_routes: routes,
        min_output_floor: minimum,
        account: state,
    };
    let submit = client.verify_and_sign_swap(&intent, &tables, bounds)?;
    Ok(client.swap_submit_in_epoch(submit, epoch, gate).await?)
}

#[cfg(test)]
#[path = "curator_tests.rs"]
mod tests;

pub(super) fn sell_quote_amount(
    input: super::router::OraclePrice,
    amount: u64,
    base: super::router::OraclePrice,
    slippage_bps: u16,
) -> Result<u64, VaultChainError> {
    Ok(
        ((u128::from(input.convert(amount, base)?) * u128::from(10_000 - slippage_bps)) / 10_000)
            as u64,
    )
}

fn missing_ata_setup(
    setup: &[solana_sdk::instruction::Instruction],
    snapshot: &BTreeMap<Pubkey, Account>,
) -> Vec<solana_sdk::instruction::Instruction> {
    setup
        .iter()
        .filter(|ix| !snapshot.contains_key(&ix.accounts[1].pubkey))
        .cloned()
        .collect()
}
