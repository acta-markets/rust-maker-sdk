use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::AddressLookupTableAccount,
    pubkey::Pubkey,
};

use super::manager::VaultManager;
use super::price_route::PriceRoute;
use super::trading::{
    NO_AMOUNT_OFFSET, VaultSaleTerms, VaultStagingAsset, VaultSwapAmountOffset, VaultSwapHop,
    VaultSwapKind, VaultSwapPlan,
};
use super::{VaultAccount, VaultChainError, pda};
use crate::chain::ix::{
    ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
    derive_associated_token_address as ata,
};
use crate::orders::{SignerLike, verify_order_id_signature_bytes};
use crate::types::{VaultLeg, VaultStagingStep, VaultSwapDirection, VaultSwapMode};
use crate::ws::types::{SponsoredTxToSignData, SwapQuoteData, SwapQuoteResultData};

const COMPUTE_BUDGET_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("ComputeBudget111111111111111111111111111111");
const ED25519_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("Ed25519SigVerify111111111111111111111111111");
const ED25519_HEADER: [u8; 16] = [
    1, 0, 48, 0, 255, 255, 16, 0, 255, 255, 112, 0, 32, 0, 255, 255,
];
const ED25519_DATA_LEN: usize = 144;
const SIGNATURE_LEN: usize = 64;
const MAX_TRANSACTION_WIRE_BYTES: usize = 1232;
const SET_COMPUTE_UNIT_LIMIT: u8 = 2;
const SET_COMPUTE_UNIT_PRICE: u8 = 3;
const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
const BPS: u128 = 10_000;
const STAGING_GENERATION_OFFSET: usize = 27;
/// SPL and Sanctum multi-validator stake-pool programs.
pub(super) const STAKE_POOL_PROGRAM_IDS: [Pubkey; 2] = [
    solana_sdk::pubkey!("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"),
    solana_sdk::pubkey!("SPMBzsVUuoHA4Jm6KunbsotaahvVikZs1JyTW6iJvbn"),
];
const UPDATE_STAKE_POOL_BALANCE: u8 = 7;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VaultIntentError {
    #[error("transaction is not valid base64")]
    InvalidBase64,
    #[error("transaction bytes are malformed: {0}")]
    Malformed(&'static str),
    #[error("unsupported transaction message version {0}")]
    UnsupportedVersion(u8),
    #[error("vault opens must use a version-0 transaction")]
    OpenRequiresVersionZero,
    #[error("vault opens must use an address lookup table")]
    OpenRequiresLookupTable,
    #[error("vault-open account {0} must be a static transaction key")]
    OpenRequiresStaticAccount(Pubkey),
    #[error("vault-open transaction is {actual} bytes, exceeding the {maximum}-byte wire limit")]
    OpenTransactionTooLarge { actual: usize, maximum: usize },
    #[error("lookup table {0} was not provided")]
    MissingLookupTable(Pubkey),
    #[error("lookup table {table} has no index {index}")]
    LookupIndex { table: Pubkey, index: u8 },
    #[error("fee payer is {found}, expected {expected}")]
    FeePayer { expected: Pubkey, found: Pubkey },
    #[error("transaction has {found} instructions, expected {expected}")]
    InstructionCount { expected: usize, found: usize },
    #[error("instruction {index} calls program {found}, expected {expected}")]
    Program {
        index: usize,
        expected: Pubkey,
        found: Pubkey,
    },
    #[error("instruction {index} has {found} accounts, expected {expected}")]
    AccountCount {
        index: usize,
        expected: usize,
        found: usize,
    },
    #[error(
        "instruction {index} account {position} is {found} (signer {found_signer}, writable {found_writable}), expected {expected} (signer {expected_signer}, writable {expected_writable})"
    )]
    Account {
        index: usize,
        position: usize,
        expected: Pubkey,
        expected_signer: bool,
        expected_writable: bool,
        found: Pubkey,
        found_signer: bool,
        found_writable: bool,
    },
    #[error("instruction {index} data differs at byte {offset}")]
    Data { index: usize, offset: usize },
    #[error("account {key} is writable {found_writable}, expected {expected_writable}")]
    Privilege {
        key: Pubkey,
        expected_writable: bool,
        found_writable: bool,
    },
    #[error("signer {0} is not one of the transaction's required signers")]
    SignerNotRequired(Pubkey),
    #[error("vault-open participant signer is {found}, expected {expected}")]
    ParticipantSigner { expected: Pubkey, found: Pubkey },
    #[error("transaction requires signer {0} outside the intent")]
    UnexpectedSigner(Pubkey),
    #[error("transaction loads account {0} that no expected instruction uses")]
    UnexpectedAccount(Pubkey),
    #[error("compute unit limit {limit} exceeds {max}")]
    ComputeUnitLimit { limit: u32, max: u32 },
    #[error("compute unit price {price} exceeds {max}")]
    ComputeUnitPrice { price: u64, max: u64 },
    #[error("compute budget instruction {0} is not a canonical limit/price instruction")]
    ComputeBudget(usize),
    #[error("maker quote signature instruction: {0}")]
    QuoteSignature(&'static str),
    #[error("served order id differs from the one the terms bind")]
    OrderId,
    #[error("swap route: {0}")]
    Route(&'static str),
    #[error("swap terms: {0}")]
    Terms(&'static str),
    #[error("served {0} price section is not the vault's price route on a configured shard")]
    PriceSection(&'static str),
    #[error("swap setup: {0}")]
    Setup(&'static str),
    #[error(transparent)]
    Intent(#[from] VaultChainError),
}

/// Ceilings for the server-chosen compute budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputeBudgetBounds {
    pub max_unit_limit: u32,
    pub max_unit_price_micro_lamports: u64,
}

/// What the signer agreed to open; the expected transaction is rebuilt from it.
#[derive(Debug, Clone, Copy)]
pub enum VaultOpenIntent<'a> {
    /// The vault writes and sells to an external maker
    /// (`OpenPositionForVaultAsTaker` or `OpenSpreadForVaultAsTaker`).
    Sale {
        /// The deployment's checked `GlobalConfig.hot_authority`; it pays and signs.
        keeper: Pubkey,
        writer: &'a VaultManager,
        maker_owner: Pubkey,
        maker_quote_signer: Pubkey,
        terms: VaultSaleTerms,
    },
    /// The vault writes against another vault's quote
    /// (`OpenPositionBetweenVaults` or `OpenSpreadBetweenVaults`).
    BetweenVaults {
        /// The deployment's checked `GlobalConfig.hot_authority`; it pays and signs.
        keeper: Pubkey,
        writer: &'a VaultManager,
        holder: &'a VaultManager,
        terms: VaultSaleTerms,
    },
    /// An external taker writes against the vault's quote
    /// (`OpenPositionForVaultAsMaker` or `OpenSpreadForVaultAsMaker`).
    Fill {
        /// The deployment's checked `GlobalConfig.hot_authority`; it pays and signs.
        keeper: Pubkey,
        holder: &'a VaultManager,
        taker: Pubkey,
        terms: VaultSaleTerms,
    },
}

/// The vault pair's price routes and the push-oracle shards the deployment posts Pyth feeds to.
#[derive(Debug, Clone, Copy)]
pub struct PairPriceRoutes<'a> {
    pub main: &'a PriceRoute,
    pub second: &'a PriceRoute,
    /// Routes of the mints the operator accepts as a staging mint; empty refuses staged swaps.
    pub staging: &'a [PriceRoute],
    pub shards: &'a [u16],
}

/// A swap the operator requested; the served price section must follow `price_routes`.
#[derive(Debug, Clone, Copy)]
pub struct VaultSwapIntent<'a> {
    pub vault: &'a VaultManager,
    pub request: &'a SwapQuoteData,
    pub quote: &'a SwapQuoteResultData,
    pub price_routes: PairPriceRoutes<'a>,
    pub min_output_floor: u64,
    /// The vault before the request; checked against its second leg and staging slot.
    pub account: VaultAccount,
}

/// Verifies a served position-open transaction against `intent` and the order id it is served for.
pub fn verify_vault_open(
    to_sign: &SponsoredTxToSignData,
    intent: &VaultOpenIntent<'_>,
    tables: &[AddressLookupTableAccount],
    bounds: ComputeBudgetBounds,
) -> Result<VerifiedTransaction, VaultIntentError> {
    let served = ServedMessage::decode(&to_sign.tx_base64, tables)?;
    served.require_vault_open_wire_shape()?;
    let (keeper, participant, quote_signer, open) = match *intent {
        VaultOpenIntent::Sale {
            keeper,
            writer,
            maker_owner,
            maker_quote_signer,
            terms,
        } => (
            keeper,
            writer.args.actor,
            maker_quote_signer,
            match terms {
                VaultSaleTerms::Physical(terms) => {
                    writer.open_position_as_taker(keeper, maker_owner, &terms)?
                }
                VaultSaleTerms::Spread(terms) => {
                    writer.open_spread_as_taker(keeper, maker_owner, &terms)?
                }
            },
        ),
        VaultOpenIntent::BetweenVaults {
            keeper,
            writer,
            holder,
            terms,
        } => (
            keeper,
            writer.args.actor,
            holder.args.delegate_authority,
            match terms {
                VaultSaleTerms::Physical(terms) => {
                    writer.open_position_with_vault_holder(keeper, holder, &terms)?
                }
                VaultSaleTerms::Spread(terms) => {
                    writer.open_spread_with_vault_holder(keeper, holder, &terms)?
                }
            },
        ),
        VaultOpenIntent::Fill {
            keeper,
            holder,
            taker,
            terms,
        } => (
            keeper,
            taker,
            holder.args.delegate_authority,
            match terms {
                VaultSaleTerms::Physical(terms) => {
                    holder.open_position_as_maker(keeper, taker, &terms)?
                }
                VaultSaleTerms::Spread(terms) => {
                    holder.open_spread_as_maker(keeper, taker, &terms)?
                }
            },
        ),
    };
    if open.order_id != *to_sign.order_id.as_bytes() {
        return Err(VaultIntentError::OrderId);
    }
    let mut static_accounts = vec![
        keeper,
        participant,
        open.addresses.position,
        open.addresses.collateral_ata,
    ];
    let collateral_ata = create_ata_idempotent(
        keeper,
        open.addresses.position,
        open.addresses.collateral_mint,
        open.addresses.collateral_token_program,
    );
    let mut expected = served.compute_budget(bounds)?;
    if let VaultOpenIntent::Fill {
        holder,
        taker,
        terms,
        ..
    } = intent
    {
        let quote_mint = match terms {
            VaultSaleTerms::Physical(terms) => terms.quote_mint,
            VaultSaleTerms::Spread(terms) => terms.quote_mint,
        };
        let (quote_mint, quote_program) = if holder.args.main_mint == quote_mint {
            (holder.args.main_mint, holder.args.main_token_program)
        } else {
            (holder.args.second_mint, holder.args.second_token_program)
        };
        expected.push(create_ata_idempotent(
            keeper,
            *taker,
            quote_mint,
            quote_program,
        ));
    }
    expected.push(collateral_ata);
    expected.push(served.quote_signature(expected.len(), quote_signer, &open.order_id)?);
    expected.push(open.instruction);
    static_accounts.extend(expected.iter().map(|instruction| instruction.program_id));
    static_accounts.extend(
        expected
            .iter()
            .flat_map(|instruction| instruction.accounts.iter())
            .filter(|account| account.is_signer)
            .map(|account| account.pubkey),
    );
    served.require_static_accounts(&static_accounts)?;
    served.matches(keeper, &expected)
}

/// Verifies a served vault swap against the operator's request and the quoted terms.
pub fn verify_vault_swap(
    intent: &VaultSwapIntent<'_>,
    tables: &[AddressLookupTableAccount],
    bounds: ComputeBudgetBounds,
) -> Result<VerifiedTransaction, VaultIntentError> {
    let VaultSwapIntent {
        vault,
        request,
        quote,
        min_output_floor,
        ..
    } = *intent;
    vault.ensure_current(&intent.account)?;
    let slot = intent.account.staging;
    if request.vault_pda != vault.addresses.vault.into() {
        return Err(VaultIntentError::Terms("request names a different vault"));
    }
    if quote.request_id != request.request_id || quote.mode != request.mode {
        return Err(VaultIntentError::Terms(
            "quote answers a different request or mode",
        ));
    }
    let kind = match (request.mode, request.direction) {
        (VaultSwapMode::Rebalance, VaultSwapDirection::MainToSecond) => VaultSwapKind::Rebalance {
            second_to_main: false,
        },
        (VaultSwapMode::Rebalance, VaultSwapDirection::SecondToMain) => VaultSwapKind::Rebalance {
            second_to_main: true,
        },
        (VaultSwapMode::Recovery, VaultSwapDirection::SecondToMain) => VaultSwapKind::Recovery,
        (VaultSwapMode::Closing, VaultSwapDirection::SecondToMain) => VaultSwapKind::Closing,
        _ => {
            return Err(VaultIntentError::Terms(
                "unsupported swap mode or direction",
            ));
        }
    };
    let amount_in = parse_amount(&quote.in_amount, "quoted input")?;
    let open_slot = slot.open;
    match (quote.staging, open_slot) {
        (Some(VaultStagingStep::Unstage), Some(slot)) if amount_in != slot.amount => {
            return Err(VaultIntentError::Terms(
                "an unstage must spend the whole staged amount",
            ));
        }
        (Some(VaultStagingStep::Unstage), None) => {
            return Err(VaultIntentError::Terms("nothing is staged to unstage"));
        }
        (Some(VaultStagingStep::Stage), Some(_)) => {
            return Err(VaultIntentError::Terms(
                "a stage needs an empty staging slot",
            ));
        }
        (Some(VaultStagingStep::Unstage), Some(_)) => {}
        _ if amount_in != parse_amount(&request.amount, "request amount")? => {
            return Err(VaultIntentError::Terms("quoted input differs from request"));
        }
        _ => {}
    }
    let quoted_out = parse_amount(&quote.quoted_out_amount, "quoted output")?;
    let minimum_out = parse_amount(&quote.minimum_out_amount, "minimum output")?;
    let slippage = u128::from(request.slippage_bps);
    if slippage > BPS
        || quoted_out == 0
        || minimum_out == 0
        || u128::from(minimum_out) < u128::from(quoted_out) * (BPS - slippage) / BPS
    {
        return Err(VaultIntentError::Terms(
            "minimum output exceeds the requested slippage",
        ));
    }
    // A stage fills the staging mint, not the output leg the floor is in.
    if quote.staging != Some(VaultStagingStep::Stage) && minimum_out < min_output_floor {
        return Err(VaultIntentError::Terms(
            "minimum output is below the operator's floor",
        ));
    }
    let valid_until = parse_amount(&quote.valid_until, "valid_until")?;

    let served = ServedMessage::decode(&quote.tx_base64, tables)?;
    let payer = vault.args.actor;
    let mut expected = served.compute_budget(bounds)?;
    while let Some(setup) = served
        .instructions
        .get(expected.len())
        .filter(|ix| ix.program_id != vault.args.program_id)
    {
        expected.push(swap_setup(vault, intent.price_routes, payer, setup)?);
    }
    let index = expected.len();
    let swap = served
        .instructions
        .get(index)
        .ok_or(VaultIntentError::InstructionCount {
            expected: index + 1,
            found: served.instructions.len(),
        })?;
    let routes = intent.price_routes;
    if routes.main.mint != vault.args.main_mint || routes.second.mint != vault.args.second_mint {
        return Err(VaultIntentError::Intent(VaultChainError::InvalidInput(
            "price routes must price the vault's main and second mints",
        )));
    }
    let a = &vault.addresses;
    let mut priced_legs = [
        ("main", a.main_price_route, routes.main),
        ("second", a.second_price_route, routes.second),
    ];
    let mut kind = match quote.staging {
        None => kind,
        Some(step) => {
            let (Some(mint), Some(token_program)) = (swap.accounts.get(9), swap.accounts.get(10))
            else {
                return Err(VaultIntentError::Route(
                    "swap instruction is shorter than its fixed layout",
                ));
            };
            let route = routes
                .staging
                .iter()
                .find(|route| route.mint == mint.pubkey)
                .ok_or(VaultIntentError::PriceSection("staging"))?;
            let expected = match (step, open_slot) {
                (VaultStagingStep::Unstage, Some(slot)) if slot.mint != mint.pubkey => {
                    return Err(VaultIntentError::Terms(
                        "unstage names another staging mint",
                    ));
                }
                (VaultStagingStep::Unstage, _) => Some(slot.generation),
                _ => slot.generation.checked_add(1),
            };
            let served = swap
                .data
                .get(STAGING_GENERATION_OFFSET..STAGING_GENERATION_OFFSET + 8)
                .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                .map(u64::from_le_bytes);
            let generation = match (expected, served) {
                (Some(expected), Some(served)) if expected == served => expected,
                _ => {
                    return Err(VaultIntentError::Terms(
                        "staging generation does not name the vault's slot",
                    ));
                }
            };
            // Stage leaves the input leg, unstage enters the output leg.
            let leg = match (step, request.direction) {
                (VaultStagingStep::Stage, VaultSwapDirection::MainToSecond)
                | (VaultStagingStep::Unstage, VaultSwapDirection::SecondToMain) => VaultLeg::Main,
                _ => VaultLeg::Second,
            };
            priced_legs = [
                priced_legs[usize::from(leg == VaultLeg::Second)],
                (
                    "staging",
                    pda::price_route(&vault.args.program_id, &route.mint).0,
                    route,
                ),
            ];
            VaultSwapKind::Staging {
                step,
                leg,
                asset: VaultStagingAsset {
                    mint: mint.pubkey,
                    token_program: token_program.pubkey,
                },
                generation,
            }
        }
    };
    let (fixed, header) = (kind.fixed_accounts(), kind.header_len());
    let (Some(priced), Some(hop_data)) = (swap.accounts.get(fixed..), swap.data.get(header..))
    else {
        return Err(VaultIntentError::Route(
            "swap instruction is shorter than its fixed layout",
        ));
    };
    let price_section = served_price_section(priced_legs, routes.shards, priced)?;
    let mut hop_accounts = &priced[price_section.len()..];
    let mut recovery_tail_len = 0;
    if request.mode == VaultSwapMode::Recovery
        && let VaultSwapKind::Staging {
            step: VaultStagingStep::Stage,
            asset,
            generation,
            ..
        } = kind
    {
        let tail = hop_accounts
            .get(2..)
            .ok_or(VaultIntentError::Route("recovery tail is truncated"))?;
        let main_price_section = served_price_section(
            [("main", a.main_price_route, routes.main)],
            routes.shards,
            tail,
        )?;
        recovery_tail_len = 2 + main_price_section.len();
        hop_accounts = &hop_accounts[recovery_tail_len..];
        kind = VaultSwapKind::RecoveryStage {
            asset,
            generation,
            main_price_section,
        };
    }
    if hop_accounts.iter().any(|meta| meta.pubkey == payer) {
        return Err(VaultIntentError::Route("route uses the fee payer"));
    }
    let share_mint = vault.args.share_mint;
    let share_ata = ata(&vault.addresses.vault, &share_mint, &TOKEN_PROGRAM_ID);
    if hop_accounts
        .iter()
        .any(|meta| meta.pubkey == share_mint || meta.pubkey == share_ata)
    {
        return Err(VaultIntentError::Route(
            "route touches the vault share mint or its share account",
        ));
    }
    let plan = VaultSwapPlan {
        price_section,
        amount_in,
        minimum_out,
        valid_until,
        hops: served_hops(swap.data[header - 1], hop_data, hop_accounts)?,
    };
    let instruction = vault
        .swap(kind.clone(), &plan)
        .map_err(|error| match error {
            VaultChainError::InvalidInput(reason) => VaultIntentError::Route(reason),
            other => VaultIntentError::Intent(other),
        })?;
    // Hops may write price or mint accounts: prices are read first, the vault is no mint authority.
    let staging = match kind {
        VaultSwapKind::Staging { asset, .. } | VaultSwapKind::RecoveryStage { asset, .. } => {
            Some(asset.mint)
        }
        _ => None,
    };
    let mint = |key: &Pubkey| {
        *key == vault.args.main_mint || *key == vault.args.second_mint || Some(*key) == staging
    };
    let mut pinned: Vec<&AccountMeta> = instruction.accounts[..fixed]
        .iter()
        .filter(|meta| !mint(&meta.pubkey))
        .collect();
    let mut position = fixed + plan.price_section.len();
    pinned.extend(&instruction.accounts[position..position + recovery_tail_len]);
    position += recovery_tail_len;
    for hop in &plan.hops {
        pinned.extend(&instruction.accounts[position..position + 3]);
        position += 3 + hop.accounts.len();
    }
    if hop_accounts.iter().any(|meta| {
        meta.is_writable
            && pinned
                .iter()
                .any(|fixed| fixed.pubkey == meta.pubkey && !fixed.is_writable)
    }) {
        return Err(VaultIntentError::Route(
            "route makes a read-only account writable",
        ));
    }
    require_route_atas(&expected, &plan.hops)?;
    expected.push(instruction);
    served.matches(payer, &expected)
}

/// Rebuilds served setup: transit/scratch ATAs or `UpdateStakePoolBalance` of a pinned pool.
fn swap_setup(
    vault: &VaultManager,
    routes: PairPriceRoutes<'_>,
    payer: Pubkey,
    served: &Instruction,
) -> Result<Instruction, VaultIntentError> {
    let key = |position: usize| {
        served
            .accounts
            .get(position)
            .map(|meta| meta.pubkey)
            .ok_or(VaultIntentError::Setup("instruction is truncated"))
    };
    let token_program = |position: usize| {
        key(position).and_then(|program| {
            if program == TOKEN_PROGRAM_ID || program == TOKEN_2022_PROGRAM_ID {
                Ok(program)
            } else {
                Err(VaultIntentError::Setup("token program is not SPL Token"))
            }
        })
    };
    if served.program_id == ASSOCIATED_TOKEN_PROGRAM_ID {
        let mint = key(3)?;
        let args = &vault.args;
        if [args.main_mint, args.second_mint, args.share_mint].contains(&mint) {
            return Err(VaultIntentError::Setup(
                "creates an account for a vault leg or share mint",
            ));
        }
        return Ok(create_ata_idempotent(
            payer,
            vault.addresses.vault,
            mint,
            token_program(5)?,
        ));
    }
    if !STAKE_POOL_PROGRAM_IDS.contains(&served.program_id) {
        return Err(VaultIntentError::Setup(
            "unexpected instruction before the swap",
        ));
    }
    let pool = key(0)?;
    let pinned = [routes.main, routes.second]
        .into_iter()
        .chain(routes.staging)
        .flat_map(PriceRoute::account_ratios)
        .any(|ratio| ratio.account == pool);
    if !pinned {
        return Err(VaultIntentError::Setup(
            "updates a stake pool the vault's routes do not pin",
        ));
    }
    Ok(update_stake_pool_balance(
        served.program_id,
        [pool, key(2)?, key(3)?, key(4)?, key(5)?, token_program(6)?],
    ))
}

pub(super) fn update_stake_pool_balance(program: Pubkey, keys: [Pubkey; 6]) -> Instruction {
    let [pool, validators, reserve, fee, mint, token_program] = keys;
    let authority = Pubkey::find_program_address(&[pool.as_ref(), b"withdraw"], &program).0;
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(pool, false),
            AccountMeta::new_readonly(authority, false),
            AccountMeta::new(validators, false),
            AccountMeta::new_readonly(reserve, false),
            AccountMeta::new(fee, false),
            AccountMeta::new(mint, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![UPDATE_STAKE_POOL_BALANCE],
    }
}

/// Lookup tables a served transaction references, to fetch before verification.
pub fn served_lookup_table_keys(tx_base64: &str) -> Result<Vec<Pubkey>, VaultIntentError> {
    let bytes = STANDARD
        .decode(tx_base64)
        .map_err(|_| VaultIntentError::InvalidBase64)?;
    Ok(CompiledMessage::decode(&bytes)?
        .lookups
        .into_iter()
        .map(|lookup| lookup.table)
        .collect())
}

/// Each leg must be its route followed by its edge accounts on one configured shard.
fn served_price_section<const N: usize>(
    legs: [(&'static str, Pubkey, &PriceRoute); N],
    shards: &[u16],
    served: &[AccountMeta],
) -> Result<Vec<AccountMeta>, VaultIntentError> {
    let mut section = Vec::new();
    for (leg, address, route) in legs {
        let rest = &served[section.len()..];
        let found = shards
            .iter()
            .map(|shard| route.accounts(address, *shard))
            .find(|expected| {
                rest.get(..expected.len()).is_some_and(|served| {
                    served
                        .iter()
                        .zip(expected)
                        .all(|(served, expected)| served.pubkey == expected.pubkey)
                })
            })
            .ok_or(VaultIntentError::PriceSection(leg))?;
        section.extend(found);
    }
    Ok(section)
}

/// Splits the served hops back out; the rebuilt instruction must match byte for byte.
fn require_route_atas(
    setup: &[Instruction],
    hops: &[VaultSwapHop],
) -> Result<(), VaultIntentError> {
    let transit = hops.split_last().map_or(&[][..], |(_, transit)| transit);
    let mut created = Vec::new();
    for ix in setup
        .iter()
        .filter(|ix| ix.program_id == ASSOCIATED_TOKEN_PROGRAM_ID)
    {
        let ata = ix.accounts[1].pubkey;
        let routed = transit.iter().any(|hop| hop.destination == ata)
            || hops
                .iter()
                .any(|hop| hop.accounts.iter().any(|meta| meta.pubkey == ata));
        if !routed {
            return Err(VaultIntentError::Setup(
                "creates an account the route does not use",
            ));
        }
        if created.contains(&ata) {
            return Err(VaultIntentError::Setup("creates the same account twice"));
        }
        created.push(ata);
    }
    Ok(())
}

fn served_hops(
    count: u8,
    mut data: &[u8],
    mut accounts: &[AccountMeta],
) -> Result<Vec<VaultSwapHop>, VaultIntentError> {
    let truncated = VaultIntentError::Route("swap hops are truncated");
    let mut hops = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let [account_count, lo, hi, offset_lo, offset_hi, rest @ ..] = data else {
            return Err(truncated);
        };
        let (amount_offset, rest) = match u16::from_le_bytes([*offset_lo, *offset_hi]) {
            NO_AMOUNT_OFFSET => (None, rest),
            offset => {
                let Some((num, rest)) = rest.split_first_chunk::<8>() else {
                    return Err(truncated);
                };
                let Some((den, rest)) = rest.split_first_chunk::<8>() else {
                    return Err(truncated);
                };
                let amount = VaultSwapAmountOffset {
                    offset,
                    num: u64::from_le_bytes(*num),
                    den: u64::from_le_bytes(*den),
                };
                (Some(amount), rest)
            }
        };
        let Some((hop_data, rest)) =
            rest.split_at_checked(usize::from(u16::from_le_bytes([*lo, *hi])))
        else {
            return Err(truncated);
        };
        let [program, source, destination, tail @ ..] = accounts else {
            return Err(truncated);
        };
        let Some((cpi, tail)) = tail.split_at_checked(usize::from(*account_count)) else {
            return Err(truncated);
        };
        hops.push(VaultSwapHop {
            program: program.pubkey,
            source: source.pubkey,
            destination: destination.pubkey,
            accounts: cpi.to_vec(),
            data: hop_data.to_vec(),
            amount_offset,
        });
        data = rest;
        accounts = tail;
    }
    Ok(hops)
}

fn parse_amount(value: &str, field: &'static str) -> Result<u64, VaultIntentError> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| VaultIntentError::Terms(field))
}

pub(super) fn create_ata_idempotent(
    payer: Pubkey,
    owner: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(
                crate::chain::ix::derive_associated_token_address(&owner, &mint, &token_program),
                false,
            ),
            AccountMeta::new_readonly(owner, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![1],
    }
}

struct Lookup {
    table: Pubkey,
    writable: Vec<u8>,
    readonly: Vec<u8>,
}

struct CompiledInstruction {
    program: u8,
    accounts: Vec<u8>,
    data: Vec<u8>,
}

struct CompiledMessage {
    signatures_start: usize,
    message_start: usize,
    required_signers: usize,
    readonly_signed: usize,
    readonly_unsigned: usize,
    static_keys: Vec<Pubkey>,
    instructions: Vec<CompiledInstruction>,
    lookups: Vec<Lookup>,
    is_version_zero: bool,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], VaultIntentError> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(VaultIntentError::Malformed("truncated transaction"))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, VaultIntentError> {
        Ok(self.take(1)?[0])
    }

    fn compact(&mut self) -> Result<usize, VaultIntentError> {
        let mut value = 0usize;
        for shift in [0u32, 7, 14] {
            let byte = self.byte()?;
            value |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(VaultIntentError::Malformed("length prefix too long"))
    }

    fn key(&mut self) -> Result<Pubkey, VaultIntentError> {
        let mut bytes = [0; 32];
        bytes.copy_from_slice(self.take(32)?);
        Ok(Pubkey::new_from_array(bytes))
    }

    fn vec(&mut self) -> Result<Vec<u8>, VaultIntentError> {
        let len = self.compact()?;
        Ok(self.take(len)?.to_vec())
    }
}

impl CompiledMessage {
    fn decode(transaction: &[u8]) -> Result<Self, VaultIntentError> {
        let mut reader = Reader {
            bytes: transaction,
            pos: 0,
        };
        let signatures = reader.compact()?;
        let signatures_start = reader.pos;
        reader.take(signatures.saturating_mul(SIGNATURE_LEN))?;
        let message_start = reader.pos;
        let mut versioned = false;
        if reader
            .bytes
            .get(reader.pos)
            .is_some_and(|byte| byte & 0x80 != 0)
        {
            let version = reader.byte()? & 0x7f;
            if version != 0 {
                return Err(VaultIntentError::UnsupportedVersion(version));
            }
            versioned = true;
        }
        let required_signers = usize::from(reader.byte()?);
        let readonly_signed = usize::from(reader.byte()?);
        let readonly_unsigned = usize::from(reader.byte()?);
        let key_count = reader.compact()?;
        let static_keys = (0..key_count)
            .map(|_| reader.key())
            .collect::<Result<Vec<_>, _>>()?;
        if signatures != required_signers
            || readonly_signed > required_signers
            || required_signers == readonly_signed
            || required_signers + readonly_unsigned > static_keys.len()
        {
            return Err(VaultIntentError::Malformed("invalid message header"));
        }
        reader.take(32)?;
        let instruction_count = reader.compact()?;
        let instructions = (0..instruction_count)
            .map(|_| {
                Ok(CompiledInstruction {
                    program: reader.byte()?,
                    accounts: reader.vec()?,
                    data: reader.vec()?,
                })
            })
            .collect::<Result<Vec<_>, VaultIntentError>>()?;
        let mut lookups = Vec::new();
        if versioned {
            for _ in 0..reader.compact()? {
                lookups.push(Lookup {
                    table: reader.key()?,
                    writable: reader.vec()?,
                    readonly: reader.vec()?,
                });
            }
        }
        if reader.pos != transaction.len() {
            return Err(VaultIntentError::Malformed("trailing bytes"));
        }
        Ok(Self {
            signatures_start,
            message_start,
            required_signers,
            readonly_signed,
            readonly_unsigned,
            static_keys,
            instructions,
            lookups,
            is_version_zero: versioned,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Flags {
    signer: bool,
    writable: bool,
}

struct ServedMessage {
    keys: Vec<(Pubkey, Flags)>,
    instructions: Vec<Instruction>,
    transaction: VerifiedTransaction,
    static_keys: HashSet<Pubkey>,
    is_version_zero: bool,
    has_lookup_table: bool,
    wire_len: usize,
}

#[derive(Debug)]
pub struct VerifiedTransaction {
    bytes: Vec<u8>,
    signatures_start: usize,
    message_start: usize,
    signers: Vec<Pubkey>,
}

impl VerifiedTransaction {
    pub fn sign(self, signer: &dyn SignerLike) -> Result<String, VaultIntentError> {
        self.sign_with_participant_signature(signer)
            .map(|(transaction, _)| transaction)
    }

    pub(crate) fn sign_with_participant_signature(
        mut self,
        signer: &dyn SignerLike,
    ) -> Result<(String, String), VaultIntentError> {
        let key = Pubkey::new_from_array(signer.pubkey_bytes());
        let index = self
            .signers
            .iter()
            .position(|required| *required == key)
            .ok_or(VaultIntentError::SignerNotRequired(key))?;
        let signature = signer.sign_message(&self.bytes[self.message_start..]);
        let slot = self.signatures_start + index * SIGNATURE_LEN;
        self.bytes[slot..slot + SIGNATURE_LEN].copy_from_slice(&signature);
        Ok((
            STANDARD.encode(self.bytes),
            crate::wire::encode_base58(&signature),
        ))
    }
}

impl ServedMessage {
    fn decode(
        tx_base64: &str,
        tables: &[AddressLookupTableAccount],
    ) -> Result<Self, VaultIntentError> {
        let bytes = STANDARD
            .decode(tx_base64)
            .map_err(|_| VaultIntentError::InvalidBase64)?;
        let wire_len = bytes.len();
        let message = CompiledMessage::decode(&bytes)?;
        let static_len = message.static_keys.len();
        let mut keys: Vec<(Pubkey, Flags)> = message
            .static_keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let flags = if index < message.required_signers {
                    Flags {
                        signer: true,
                        writable: index < message.required_signers - message.readonly_signed,
                    }
                } else {
                    Flags {
                        signer: false,
                        writable: index < static_len - message.readonly_unsigned,
                    }
                };
                (*key, flags)
            })
            .collect();
        for writable in [true, false] {
            for lookup in &message.lookups {
                let table = tables
                    .iter()
                    .find(|table| table.key == lookup.table)
                    .ok_or(VaultIntentError::MissingLookupTable(lookup.table))?;
                let indexes = if writable {
                    &lookup.writable
                } else {
                    &lookup.readonly
                };
                for index in indexes {
                    let key = table.addresses.get(usize::from(*index)).ok_or(
                        VaultIntentError::LookupIndex {
                            table: lookup.table,
                            index: *index,
                        },
                    )?;
                    keys.push((
                        *key,
                        Flags {
                            signer: false,
                            writable,
                        },
                    ));
                }
            }
        }
        let mut seen = HashSet::with_capacity(keys.len());
        if !keys.iter().all(|(key, _)| seen.insert(*key)) {
            return Err(VaultIntentError::Malformed("duplicate account key"));
        }
        let instructions = message
            .instructions
            .iter()
            .map(|compiled| {
                let key = |index: u8| {
                    keys.get(usize::from(index))
                        .ok_or(VaultIntentError::Malformed("account index out of range"))
                };
                let (program_id, _) = key(compiled.program)?;
                let accounts = compiled
                    .accounts
                    .iter()
                    .map(|index| {
                        key(*index).map(|(pubkey, flags)| AccountMeta {
                            pubkey: *pubkey,
                            is_signer: flags.signer,
                            is_writable: flags.writable,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Instruction {
                    program_id: *program_id,
                    accounts,
                    data: compiled.data.clone(),
                })
            })
            .collect::<Result<Vec<_>, VaultIntentError>>()?;
        let transaction = VerifiedTransaction {
            signatures_start: message.signatures_start,
            message_start: message.message_start,
            signers: message.static_keys[..message.required_signers].to_vec(),
            bytes,
        };
        Ok(Self {
            keys,
            instructions,
            transaction,
            static_keys: message.static_keys.into_iter().collect(),
            is_version_zero: message.is_version_zero,
            has_lookup_table: message
                .lookups
                .iter()
                .any(|lookup| !lookup.writable.is_empty() || !lookup.readonly.is_empty()),
            wire_len,
        })
    }

    fn require_vault_open_wire_shape(&self) -> Result<(), VaultIntentError> {
        if !self.is_version_zero {
            return Err(VaultIntentError::OpenRequiresVersionZero);
        }
        if !self.has_lookup_table {
            return Err(VaultIntentError::OpenRequiresLookupTable);
        }
        if self.wire_len > MAX_TRANSACTION_WIRE_BYTES {
            return Err(VaultIntentError::OpenTransactionTooLarge {
                actual: self.wire_len,
                maximum: MAX_TRANSACTION_WIRE_BYTES,
            });
        }
        Ok(())
    }

    fn require_static_accounts(&self, accounts: &[Pubkey]) -> Result<(), VaultIntentError> {
        accounts
            .iter()
            .find(|account| !self.static_keys.contains(account))
            .copied()
            .map_or(Ok(()), |account| {
                Err(VaultIntentError::OpenRequiresStaticAccount(account))
            })
    }

    fn compute_budget(
        &self,
        bounds: ComputeBudgetBounds,
    ) -> Result<Vec<Instruction>, VaultIntentError> {
        let budget = |index: usize, tag: u8, len: usize| {
            self.instructions
                .get(index)
                .filter(|ix| {
                    ix.program_id == COMPUTE_BUDGET_PROGRAM_ID
                        && ix.accounts.is_empty()
                        && ix.data.len() == len
                        && ix.data.first() == Some(&tag)
                })
                .map(|ix| &ix.data[1..])
                .ok_or(VaultIntentError::ComputeBudget(index))
        };
        let mut limit = [0; 4];
        limit.copy_from_slice(budget(0, SET_COMPUTE_UNIT_LIMIT, 5)?);
        let limit = u32::from_le_bytes(limit);
        let mut price = [0; 8];
        price.copy_from_slice(budget(1, SET_COMPUTE_UNIT_PRICE, 9)?);
        let price = u64::from_le_bytes(price);
        let max_limit = bounds.max_unit_limit.min(MAX_COMPUTE_UNIT_LIMIT);
        if limit == 0 || limit > max_limit {
            return Err(VaultIntentError::ComputeUnitLimit {
                limit,
                max: max_limit,
            });
        }
        if price > bounds.max_unit_price_micro_lamports {
            return Err(VaultIntentError::ComputeUnitPrice {
                price,
                max: bounds.max_unit_price_micro_lamports,
            });
        }
        let instruction = |tag: u8, value: &[u8]| {
            let mut data = Vec::with_capacity(1 + value.len());
            data.push(tag);
            data.extend_from_slice(value);
            Instruction {
                program_id: COMPUTE_BUDGET_PROGRAM_ID,
                accounts: Vec::new(),
                data,
            }
        };
        Ok(vec![
            instruction(SET_COMPUTE_UNIT_LIMIT, &limit.to_le_bytes()),
            instruction(SET_COMPUTE_UNIT_PRICE, &price.to_le_bytes()),
        ])
    }

    fn quote_signature(
        &self,
        index: usize,
        signer: Pubkey,
        order_id: &[u8; 32],
    ) -> Result<Instruction, VaultIntentError> {
        let ix = self
            .instructions
            .get(index)
            .ok_or(VaultIntentError::QuoteSignature("missing"))?;
        if ix.program_id != ED25519_PROGRAM_ID
            || !ix.accounts.is_empty()
            || ix.data.len() != ED25519_DATA_LEN
            || ix.data[..16] != ED25519_HEADER
        {
            return Err(VaultIntentError::QuoteSignature(
                "not a single inline Ed25519 check",
            ));
        }
        if ix.data[16..48] != signer.to_bytes() {
            return Err(VaultIntentError::QuoteSignature(
                "binds a different quote signer",
            ));
        }
        if ix.data[112..] != *order_id {
            return Err(VaultIntentError::QuoteSignature("binds a different order"));
        }
        let mut signature = [0; SIGNATURE_LEN];
        signature.copy_from_slice(&ix.data[48..112]);
        verify_order_id_signature_bytes(order_id, &signature, &signer.to_bytes())
            .map_err(|_| VaultIntentError::QuoteSignature("signature does not verify"))?;
        Ok(ix.clone())
    }

    fn matches(
        self,
        payer: Pubkey,
        expected: &[Instruction],
    ) -> Result<VerifiedTransaction, VaultIntentError> {
        let mut flags: HashMap<Pubkey, Flags> = HashMap::new();
        flags.insert(
            payer,
            Flags {
                signer: true,
                writable: true,
            },
        );
        for ix in expected {
            flags.entry(ix.program_id).or_insert(Flags {
                signer: false,
                writable: false,
            });
            for meta in &ix.accounts {
                let entry = flags.entry(meta.pubkey).or_insert(Flags {
                    signer: false,
                    writable: false,
                });
                entry.signer |= meta.is_signer;
                entry.writable |= meta.is_writable;
            }
        }
        if let Some((found, _)) = self.keys.first().filter(|(key, _)| *key != payer) {
            return Err(VaultIntentError::FeePayer {
                expected: payer,
                found: *found,
            });
        }
        if self.instructions.len() != expected.len() {
            return Err(VaultIntentError::InstructionCount {
                expected: expected.len(),
                found: self.instructions.len(),
            });
        }
        for (index, (served, expected)) in self.instructions.iter().zip(expected).enumerate() {
            if served.program_id != expected.program_id {
                return Err(VaultIntentError::Program {
                    index,
                    expected: expected.program_id,
                    found: served.program_id,
                });
            }
            if served.accounts.len() != expected.accounts.len() {
                return Err(VaultIntentError::AccountCount {
                    index,
                    expected: expected.accounts.len(),
                    found: served.accounts.len(),
                });
            }
            for (position, (found, wanted)) in
                served.accounts.iter().zip(&expected.accounts).enumerate()
            {
                let wanted_flags = match flags.get(&wanted.pubkey) {
                    Some(flags) => *flags,
                    None => Flags {
                        signer: wanted.is_signer,
                        writable: wanted.is_writable,
                    },
                };
                if found.pubkey != wanted.pubkey
                    || found.is_signer != wanted_flags.signer
                    || found.is_writable != wanted_flags.writable
                {
                    return Err(VaultIntentError::Account {
                        index,
                        position,
                        expected: wanted.pubkey,
                        expected_signer: wanted_flags.signer,
                        expected_writable: wanted_flags.writable,
                        found: found.pubkey,
                        found_signer: found.is_signer,
                        found_writable: found.is_writable,
                    });
                }
            }
            if served.data != expected.data {
                let offset = match served
                    .data
                    .iter()
                    .zip(&expected.data)
                    .position(|(a, b)| a != b)
                {
                    Some(offset) => offset,
                    None => served.data.len().min(expected.data.len()),
                };
                return Err(VaultIntentError::Data { index, offset });
            }
        }
        for (key, found) in &self.keys {
            match flags.get(key) {
                None => return Err(VaultIntentError::UnexpectedAccount(*key)),
                Some(wanted) if found.signer && !wanted.signer => {
                    return Err(VaultIntentError::UnexpectedSigner(*key));
                }
                Some(wanted) if found != wanted => {
                    return Err(VaultIntentError::Privilege {
                        key: *key,
                        expected_writable: wanted.writable,
                        found_writable: found.writable,
                    });
                }
                Some(_) => {}
            }
        }
        Ok(self.transaction)
    }
}

#[cfg(test)]
#[path = "served_tests.rs"]
mod tests;
