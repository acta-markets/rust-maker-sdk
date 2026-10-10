use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::AddressLookupTableAccount,
    pubkey::Pubkey,
};

use super::VaultChainError;
use super::manager::VaultManager;
use super::trading::{VaultSwapKind, VaultSwapRoute, VaultTradeTerms};
use crate::chain::ix::{ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID};
use crate::orders::verify_order_id_signature_bytes;
use crate::types::{VaultSwapDirection, VaultSwapMode};
use crate::ws::types::{SwapQuoteData, SwapQuoteResultData};

const COMPUTE_BUDGET_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("ComputeBudget111111111111111111111111111111");
const ED25519_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("Ed25519SigVerify111111111111111111111111111");
const ED25519_HEADER: [u8; 16] = [
    1, 0, 48, 0, 255, 255, 16, 0, 255, 255, 112, 0, 32, 0, 255, 255,
];
const ED25519_DATA_LEN: usize = 144;
const SET_COMPUTE_UNIT_LIMIT: u8 = 2;
const SET_COMPUTE_UNIT_PRICE: u8 = 3;
const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
const BPS: u128 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VaultIntentError {
    #[error("transaction is not valid base64")]
    InvalidBase64,
    #[error("transaction bytes are malformed: {0}")]
    Malformed(&'static str),
    #[error("unsupported transaction message version {0}")]
    UnsupportedVersion(u8),
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
    /// The vault writes and sells to an external maker (`OpenPositionForVaultAsTaker`).
    Sale {
        writer: &'a VaultManager,
        maker_owner: Pubkey,
        maker_quote_signer: Pubkey,
        terms: VaultTradeTerms,
    },
    /// The vault writes against another vault's quote (`OpenPositionBetweenVaults`).
    BetweenVaults {
        writer: &'a VaultManager,
        holder: &'a VaultManager,
        terms: VaultTradeTerms,
    },
    /// An external taker writes against the vault's quote (`OpenPositionForVaultAsMaker`).
    Fill {
        holder: &'a VaultManager,
        taker: Pubkey,
        terms: VaultTradeTerms,
    },
}

/// A swap the operator requested, plus the oracle price updates its vault reads.
#[derive(Debug, Clone, Copy)]
pub struct VaultSwapIntent<'a> {
    pub vault: &'a VaultManager,
    pub request: &'a SwapQuoteData,
    pub quote: &'a SwapQuoteResultData,
    pub main_price_update: Pubkey,
    pub second_price_update: Pubkey,
}

/// Verifies a served position-open transaction against `intent`; returns the bound order id.
pub fn verify_vault_open(
    tx_base64: &str,
    intent: &VaultOpenIntent<'_>,
    tables: &[AddressLookupTableAccount],
    bounds: ComputeBudgetBounds,
) -> Result<[u8; 32], VaultIntentError> {
    let served = ServedMessage::decode(tx_base64, tables)?;
    let (payer, quote_signer, open) = match *intent {
        VaultOpenIntent::Sale {
            writer,
            maker_owner,
            maker_quote_signer,
            terms,
        } => (
            writer.args.actor,
            maker_quote_signer,
            writer.open_position_as_taker(maker_owner, &terms)?,
        ),
        VaultOpenIntent::BetweenVaults {
            writer,
            holder,
            terms,
        } => (
            writer.args.actor,
            holder.args.delegate_authority,
            writer.open_position_with_vault_holder(holder, &terms)?,
        ),
        VaultOpenIntent::Fill {
            holder,
            taker,
            terms,
        } => (
            taker,
            holder.args.delegate_authority,
            holder.open_position_as_maker(taker, &terms)?,
        ),
    };
    let collateral_ata = create_ata_idempotent(
        payer,
        open.addresses.position,
        open.addresses.collateral_mint,
        open.addresses.collateral_token_program,
    );
    let mut expected = served.compute_budget(bounds)?;
    if let VaultOpenIntent::Fill { holder, terms, .. } = intent {
        let (quote_mint, quote_program) = if holder.args.main_mint == terms.quote_mint {
            (holder.args.main_mint, holder.args.main_token_program)
        } else {
            (holder.args.second_mint, holder.args.second_token_program)
        };
        expected.push(create_ata_idempotent(
            payer,
            payer,
            quote_mint,
            quote_program,
        ));
    }
    expected.push(collateral_ata);
    expected.push(served.quote_signature(expected.len(), quote_signer, &open.order_id)?);
    expected.push(open.instruction);
    served.matches(payer, &expected)?;
    Ok(open.order_id)
}

/// Verifies a served vault swap against the operator's request and the quoted terms.
pub fn verify_vault_swap(
    intent: &VaultSwapIntent<'_>,
    tables: &[AddressLookupTableAccount],
    bounds: ComputeBudgetBounds,
) -> Result<(), VaultIntentError> {
    let VaultSwapIntent {
        vault,
        request,
        quote,
        ..
    } = *intent;
    if request.vault_pda != vault.addresses.vault.to_string() {
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
    let amount_in = parse_amount(&request.amount, "request amount")?;
    if parse_amount(&quote.in_amount, "quoted input")? != amount_in {
        return Err(VaultIntentError::Terms("quoted input differs from request"));
    }
    let quoted_out = parse_amount(&quote.quoted_out_amount, "quoted output")?;
    let minimum_out = parse_amount(&quote.minimum_out_amount, "minimum output")?;
    let slippage = u128::from(request.slippage_bps);
    if slippage > BPS
        || quoted_out == 0
        || minimum_out == 0
        || u128::from(minimum_out) * BPS < u128::from(quoted_out) * (BPS - slippage)
    {
        return Err(VaultIntentError::Terms(
            "minimum output exceeds the requested slippage",
        ));
    }
    let valid_until = parse_amount(&quote.valid_until, "valid_until")?;

    let served = ServedMessage::decode(&quote.tx_base64, tables)?;
    let payer = vault.args.actor;
    let mut expected = served.compute_budget(bounds)?;
    let index = expected.len();
    let swap = served
        .instructions
        .get(index)
        .ok_or(VaultIntentError::InstructionCount {
            expected: index + 1,
            found: served.instructions.len(),
        })?;
    let fixed = kind.fixed_accounts();
    let header = match kind {
        VaultSwapKind::Rebalance { .. } => 28,
        VaultSwapKind::Recovery | VaultSwapKind::Closing => 27,
    };
    let (Some(route_accounts), Some(route_data)) =
        (swap.accounts.get(fixed..), swap.data.get(header..))
    else {
        return Err(VaultIntentError::Route(
            "swap instruction is shorter than its fixed layout",
        ));
    };
    let route = VaultSwapRoute {
        main_price_update: intent.main_price_update,
        second_price_update: intent.second_price_update,
        amount_in,
        minimum_out,
        valid_until,
        route_accounts: route_accounts.to_vec(),
        route_data: route_data.to_vec(),
    };
    let instruction = vault.swap(kind, &route).map_err(|error| match error {
        VaultChainError::InvalidInput(reason) => VaultIntentError::Route(reason),
        other => VaultIntentError::Intent(other),
    })?;
    let fixed_accounts = &instruction.accounts[..fixed];
    for meta in route_accounts {
        if meta.pubkey == payer {
            return Err(VaultIntentError::Route("route uses the fee payer"));
        }
        if meta.is_writable
            && fixed_accounts
                .iter()
                .any(|fixed| fixed.pubkey == meta.pubkey && !fixed.is_writable)
        {
            return Err(VaultIntentError::Route(
                "route makes a read-only vault account writable",
            ));
        }
    }
    expected.push(instruction);
    served.matches(payer, &expected)
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

fn parse_amount(value: &str, field: &'static str) -> Result<u64, VaultIntentError> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| VaultIntentError::Terms(field))
}

fn create_ata_idempotent(
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
    required_signers: usize,
    readonly_signed: usize,
    readonly_unsigned: usize,
    static_keys: Vec<Pubkey>,
    instructions: Vec<CompiledInstruction>,
    lookups: Vec<Lookup>,
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
        let bytes: [u8; 32] = self
            .take(32)?
            .try_into()
            .map_err(|_| VaultIntentError::Malformed("truncated account key"))?;
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
        reader.take(signatures.saturating_mul(64))?;
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
            required_signers,
            readonly_signed,
            readonly_unsigned,
            static_keys,
            instructions,
            lookups,
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
}

impl ServedMessage {
    fn decode(
        tx_base64: &str,
        tables: &[AddressLookupTableAccount],
    ) -> Result<Self, VaultIntentError> {
        let bytes = STANDARD
            .decode(tx_base64)
            .map_err(|_| VaultIntentError::InvalidBase64)?;
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
        Ok(Self { keys, instructions })
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
                .map(|ix| ix.data[1..].to_vec())
                .ok_or(VaultIntentError::ComputeBudget(index))
        };
        let limit = budget(0, SET_COMPUTE_UNIT_LIMIT, 5)?;
        let price = budget(1, SET_COMPUTE_UNIT_PRICE, 9)?;
        let limit = u32::from_le_bytes(
            limit
                .try_into()
                .map_err(|_| VaultIntentError::ComputeBudget(0))?,
        );
        let price = u64::from_le_bytes(
            price
                .try_into()
                .map_err(|_| VaultIntentError::ComputeBudget(1))?,
        );
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
        let signature: [u8; 64] = ix.data[48..112]
            .try_into()
            .map_err(|_| VaultIntentError::QuoteSignature("truncated signature"))?;
        verify_order_id_signature_bytes(order_id, &signature, &signer.to_bytes())
            .map_err(|_| VaultIntentError::QuoteSignature("signature does not verify"))?;
        Ok(ix.clone())
    }

    fn matches(&self, payer: Pubkey, expected: &[Instruction]) -> Result<(), VaultIntentError> {
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
        Ok(())
    }
}

#[cfg(test)]
#[path = "served_tests.rs"]
mod tests;
