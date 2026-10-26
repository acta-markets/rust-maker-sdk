use solana_sdk::{account::Account, pubkey::Pubkey};

use super::ChainError;
use crate::types::{Decimals, OrderId, PositionType, Quantity, Strike};

const ACCOUNT_OFFSET_DISCRIMINATOR: usize = 0;
const ACCOUNT_OFFSET_VERSION: usize = 1;
pub(crate) const POSITION_DISCRIMINATOR: u8 = 1;
const MAKER_DISCRIMINATOR: u8 = 2;
const MARKET_DISCRIMINATOR: u8 = 4;
const ACCOUNT_VERSION: u8 = 1;

/// On-chain position status discriminant.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionStatus {
    None = 0,
    Open = 1,
    Funded = 2,
    Liquidated = 3,
    Settled = 4,
    Closed = 5,
}

impl TryFrom<u8> for PositionStatus {
    type Error = ChainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Open),
            2 => Ok(Self::Funded),
            3 => Ok(Self::Liquidated),
            4 => Ok(Self::Settled),
            5 => Ok(Self::Closed),
            other => Err(ChainError::UnknownPositionStatus(other)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PositionInfo {
    pub position_type: PositionType,
    pub flags: u8,
    pub flags2: u8,
    pub flags3: u8,
    pub status: PositionStatus,
    pub taker_owner: Pubkey,
    pub maker_owner: Pubkey,
    pub market_pda: Pubkey,
    pub strike: Strike,
    pub quantity: Quantity,
    pub order_id: OrderId,
}

#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub expiry_ts: u64,
    pub is_put: bool,
    pub is_finalized: bool,
    pub settlement_price: u64,
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub underlying_token_program_id: Pubkey,
    pub quote_token_program_id: Pubkey,
    pub underlying_decimals: Decimals,
    pub quote_decimals: Decimals,
}

const POSITION_OFFSET_POSITION_TYPE: usize = 3;
const POSITION_OFFSET_STATUS: usize = 4;
const POSITION_OFFSET_TAKER_OWNER: usize = 8;
pub(crate) const POSITION_OFFSET_MAKER_OWNER: usize = 40;
const POSITION_OFFSET_MARKET: usize = 72;
const POSITION_OFFSET_STRIKE: usize = 104;
const POSITION_OFFSET_QUANTITY: usize = 112;
const POSITION_OFFSET_ORDER_ID: usize = 128;
pub(crate) const POSITION_LEN: usize = 424;
const POSITION_VERSION: u8 = 2;

impl PositionInfo {
    pub fn from_account(account: &Account, program_id: &Pubkey) -> Result<Self, ChainError> {
        current_layout(
            account,
            program_id,
            POSITION_DISCRIMINATOR,
            POSITION_VERSION,
            POSITION_LEN,
        )?;
        let mut order_id = [0u8; 32];
        order_id.copy_from_slice(
            &account.data[POSITION_OFFSET_ORDER_ID..POSITION_OFFSET_ORDER_ID + 32],
        );
        Ok(Self {
            position_type: PositionType::try_from(account.data[POSITION_OFFSET_POSITION_TYPE])
                .map_err(|error| ChainError::InvalidPositionType(error.0))?,
            flags: account.data[5],
            flags2: account.data[6],
            flags3: account.data[7],
            status: PositionStatus::try_from(account.data[POSITION_OFFSET_STATUS])?,
            taker_owner: read_pubkey(&account.data, POSITION_OFFSET_TAKER_OWNER)?,
            maker_owner: read_pubkey(&account.data, POSITION_OFFSET_MAKER_OWNER)?,
            market_pda: read_pubkey(&account.data, POSITION_OFFSET_MARKET)?,
            strike: Strike::new(read_u64(&account.data, POSITION_OFFSET_STRIKE)?),
            quantity: Quantity::new(read_u64(&account.data, POSITION_OFFSET_QUANTITY)?),
            order_id: OrderId::new(order_id),
        })
    }
}

const MARKET_OFFSET_UNDERLYING_DECIMALS: usize = 3;
const MARKET_OFFSET_QUOTE_DECIMALS: usize = 4;
const MARKET_OFFSET_FLAGS: usize = 5;
const MARKET_OFFSET_EXPIRY: usize = 8;
const MARKET_OFFSET_UNDERLYING_MINT: usize = 32;
const MARKET_OFFSET_QUOTE_MINT: usize = 64;
const MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM: usize = 96;
const MARKET_OFFSET_QUOTE_TOKEN_PROGRAM: usize = 128;
const MARKET_LEN: usize = 416;
const MARKET_FLAG_PUT: u8 = 1;
const MARKET_FLAG_FINALIZED: u8 = 2;

impl MarketInfo {
    /// Decodes a market account owned by `program_id` with the current header and layout.
    pub fn from_account(account: &Account, program_id: &Pubkey) -> Result<Self, ChainError> {
        let data = current_layout(
            account,
            program_id,
            MARKET_DISCRIMINATOR,
            ACCOUNT_VERSION,
            MARKET_LEN,
        )?;
        let flags = data[MARKET_OFFSET_FLAGS];
        if flags & !(MARKET_FLAG_PUT | MARKET_FLAG_FINALIZED) != 0 {
            return Err(ChainError::InvalidAccountData);
        }
        Ok(Self {
            expiry_ts: read_u64(data, MARKET_OFFSET_EXPIRY)?,
            is_put: flags & MARKET_FLAG_PUT != 0,
            is_finalized: flags & MARKET_FLAG_FINALIZED != 0,
            settlement_price: read_u64(data, 16)?,
            underlying_decimals: Decimals::new(data[MARKET_OFFSET_UNDERLYING_DECIMALS]),
            quote_decimals: Decimals::new(data[MARKET_OFFSET_QUOTE_DECIMALS]),
            underlying_mint: read_pubkey(data, MARKET_OFFSET_UNDERLYING_MINT)?,
            quote_mint: read_pubkey(data, MARKET_OFFSET_QUOTE_MINT)?,
            underlying_token_program_id: read_pubkey(data, MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM)?,
            quote_token_program_id: read_pubkey(data, MARKET_OFFSET_QUOTE_TOKEN_PROGRAM)?,
        })
    }
}

/// The maker account's identity: its owner and registered quote-signing key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MakerInfo {
    pub owner: Pubkey,
    pub quote_signer: Pubkey,
}

const MAKER_OFFSET_OWNER: usize = 4;
const MAKER_OFFSET_QUOTE_SIGNER: usize = 36;
const MAKER_LEN: usize = 328;

impl MakerInfo {
    /// Decodes a maker account owned by `program_id` with the current header and layout.
    pub fn from_account(account: &Account, program_id: &Pubkey) -> Result<Self, ChainError> {
        let data = current_layout(
            account,
            program_id,
            MAKER_DISCRIMINATOR,
            ACCOUNT_VERSION,
            MAKER_LEN,
        )?;
        Ok(Self {
            owner: read_pubkey(data, MAKER_OFFSET_OWNER)?,
            quote_signer: read_pubkey(data, MAKER_OFFSET_QUOTE_SIGNER)?,
        })
    }
}

fn current_layout<'a>(
    account: &'a Account,
    program_id: &Pubkey,
    discriminator: u8,
    version: u8,
    len: usize,
) -> Result<&'a [u8], ChainError> {
    validate_account(account, program_id, discriminator)?;
    if account.data.len() != len || account.data[ACCOUNT_OFFSET_VERSION] != version {
        return Err(ChainError::InvalidAccountData);
    }
    Ok(&account.data)
}

fn validate_account(
    account: &Account,
    program_id: &Pubkey,
    expected_discriminator: u8,
) -> Result<(), ChainError> {
    if account.owner != *program_id {
        return Err(ChainError::AccountOwnerMismatch {
            expected: *program_id,
            actual: account.owner,
        });
    }
    let discriminator = account
        .data
        .get(ACCOUNT_OFFSET_DISCRIMINATOR)
        .copied()
        .ok_or(ChainError::InvalidAccountData)?;
    if discriminator != expected_discriminator {
        return Err(ChainError::AccountDiscriminatorMismatch {
            expected: expected_discriminator,
            actual: discriminator,
        });
    }
    match account.data.get(ACCOUNT_OFFSET_VERSION) {
        None | Some(0) => return Err(ChainError::UninitializedAccount),
        Some(_) => {}
    }
    Ok(())
}

fn read_pubkey(data: &[u8], offset: usize) -> Result<Pubkey, ChainError> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .ok_or(ChainError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ChainError::InvalidAccountData)?;
    Ok(Pubkey::new_from_array(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ChainError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(ChainError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ChainError::InvalidAccountData)?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
