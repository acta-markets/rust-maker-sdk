use super::{
    super::{VaultChainError, VaultSwapAmountOffset, VaultSwapHop},
    invalid,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use solana_sdk::{instruction::AccountMeta, message::AddressLookupTableAccount, pubkey::Pubkey};
use std::collections::BTreeMap;

pub const PROGRAM: Pubkey = solana_sdk::pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
pub const EXACT_IN_SELECTORS: [[u8; 8]; 4] = [
    [0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a],
    [0xc1, 0x20, 0x9b, 0x33, 0x41, 0xd6, 0x9c, 0x81],
    [187, 100, 250, 204, 49, 196, 175, 20],
    [209, 152, 83, 147, 124, 254, 216, 233],
];

/// Matches the contract SDK's Jupiter registry preset, including ExactOut variants.
pub const VENUE_SELECTORS: [[u8; 8]; 8] = [
    EXACT_IN_SELECTORS[0],
    EXACT_IN_SELECTORS[1],
    [208, 51, 239, 151, 123, 43, 237, 92],
    [176, 209, 105, 168, 154, 125, 69, 62],
    EXACT_IN_SELECTORS[2],
    EXACT_IN_SELECTORS[3],
    [157, 138, 184, 82, 21, 244, 243, 36],
    [53, 96, 229, 202, 216, 187, 250, 24],
];

#[derive(Debug, Clone)]
pub struct Request {
    pub owner: Pubkey,
    pub payer: Pubkey,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub source: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
    pub slippage_bps: u16,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResponse {
    pub in_amount: String,
    pub out_amount: String,
    pub other_amount_threshold: String,
    pub slippage_bps: u16,
    pub swap_mode: Option<String>,
    pub swap_instruction: WireInstruction,
    pub cleanup_instruction: Option<WireInstruction>,
    pub tip_instruction: Option<WireInstruction>,
    #[serde(default)]
    pub other_instructions: Vec<WireInstruction>,
    #[serde(default)]
    pub addresses_by_lookup_table_address: Option<BTreeMap<String, Vec<String>>>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireInstruction {
    pub program_id: String,
    pub accounts: Vec<WireAccount>,
    pub data: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireAccount {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
}
#[derive(Debug, Clone)]
pub struct Quote {
    pub request: Request,
    pub output: u64,
    pub minimum: u64,
    pub hop: VaultSwapHop,
    pub tables: Vec<AddressLookupTableAccount>,
}
impl BuildResponse {
    pub fn into_quote(self, request: Request) -> Result<Quote, VaultChainError> {
        let output = parse_amount(&self.out_amount)?;
        let minimum = parse_amount(&self.other_amount_threshold)?;
        if request.amount == 0
            || parse_amount(&self.in_amount)? != request.amount
            || self.slippage_bps != request.slippage_bps
            || self.swap_mode.as_deref().is_some_and(|m| m != "ExactIn")
            || minimum == 0
            || minimum > output
            || u128::from(minimum)
                < u128::from(output) * u128::from(10_000u16.saturating_sub(request.slippage_bps))
                    / 10_000
        {
            return Err(invalid("Jupiter quote does not match ExactIn request"));
        }
        if self.cleanup_instruction.is_some()
            || self.tip_instruction.is_some()
            || !self.other_instructions.is_empty()
        {
            return Err(invalid("unsupported Jupiter side instruction"));
        }
        let wire = self.swap_instruction;
        if key(&wire.program_id)? != PROGRAM {
            return Err(invalid("unexpected Jupiter program"));
        }
        let data = STANDARD
            .decode(wire.data)
            .map_err(|_| invalid("invalid Jupiter instruction base64"))?;
        let at = input_offset(&data)?;
        if u64::from_le_bytes(
            data[at..at + 8]
                .try_into()
                .map_err(|_| invalid("Jupiter amount missing"))?,
        ) != request.amount
        {
            return Err(invalid("Jupiter instruction input differs from quote"));
        }
        let accounts = wire
            .accounts
            .into_iter()
            .map(|a| {
                Ok(AccountMeta {
                    pubkey: key(&a.pubkey)?,
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
            })
            .collect::<Result<Vec<_>, VaultChainError>>()?;
        let tables = self
            .addresses_by_lookup_table_address
            .into_iter()
            .flatten()
            .map(|(k, v)| {
                Ok(AddressLookupTableAccount {
                    key: key(&k)?,
                    addresses: v.iter().map(|s| key(s)).collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<_, VaultChainError>>()?;
        let hop = VaultSwapHop {
            program: PROGRAM,
            source: request.source,
            destination: request.destination,
            accounts,
            data,
            amount_offset: None,
        };
        Ok(Quote {
            request,
            output,
            minimum,
            hop,
            tables,
        })
    }
}
impl Quote {
    pub fn hop(&self, patch: bool) -> Result<VaultSwapHop, VaultChainError> {
        let mut hop = self.hop.clone();
        if patch {
            hop.amount_offset = Some(VaultSwapAmountOffset {
                offset: u16::try_from(input_offset(&hop.data)?)
                    .map_err(|_| invalid("Jupiter data too large"))?,
                num: 1,
                den: 1,
            });
        }
        Ok(hop)
    }
}
fn key(s: &str) -> Result<Pubkey, VaultChainError> {
    s.parse().map_err(|_| invalid("invalid Jupiter public key"))
}
fn parse_amount(s: &str) -> Result<u64, VaultChainError> {
    s.parse().map_err(|_| invalid("invalid Jupiter amount"))
}

fn input_offset(data: &[u8]) -> Result<usize, VaultChainError> {
    let kind = EXACT_IN_SELECTORS
        .iter()
        .position(|s| data.starts_with(s))
        .ok_or(invalid("unsupported Jupiter ExactIn selector"))?;
    let minimum = [31, 32, 34, 35][kind];
    if data.len() < minimum {
        return Err(invalid("truncated Jupiter ExactIn data"));
    }
    // Legacy route_plan precedes the fixed 19-byte suffix; v2 puts amounts before route_plan.
    Ok(match kind {
        0 | 1 => data.len() - 19,
        2 => 8,
        3 => 9,
        _ => return Err(invalid("unsupported Jupiter ExactIn selector")),
    })
}
