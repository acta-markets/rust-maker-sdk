use solana_sdk::{account::Account, pubkey::Pubkey};

use super::ChainError;
use crate::types::{Decimals, OrderId, PositionType, Quantity, Strike};

const ACCOUNT_OFFSET_DISCRIMINATOR: usize = 0;
const ACCOUNT_OFFSET_VERSION: usize = 1;
const POSITION_DISCRIMINATOR: u8 = 1;
const MARKET_DISCRIMINATOR: u8 = 4;

/// On-chain position status discriminant.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionStatus {
    None = 0,
    Open = 1,
    Funded = 2,
    Liquidated = 3,
    Settled = 4,
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
            other => Err(ChainError::UnknownPositionStatus(other)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PositionInfo {
    pub position_type: PositionType,
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
const POSITION_OFFSET_MAKER_OWNER: usize = 40;
const POSITION_OFFSET_MARKET: usize = 72;
const POSITION_OFFSET_STRIKE: usize = 104;
const POSITION_OFFSET_QUANTITY: usize = 112;
const POSITION_OFFSET_ORDER_ID: usize = 128;
const POSITION_MIN_LEN: usize = POSITION_OFFSET_ORDER_ID + 32;

pub(crate) fn parse_position(
    account: &Account,
    program_id: &Pubkey,
) -> Result<PositionInfo, ChainError> {
    validate_account(account, program_id, POSITION_DISCRIMINATOR)?;
    if account.data.len() < POSITION_MIN_LEN {
        return Err(ChainError::InvalidAccountData);
    }
    let mut order_id = [0u8; 32];
    order_id
        .copy_from_slice(&account.data[POSITION_OFFSET_ORDER_ID..POSITION_OFFSET_ORDER_ID + 32]);
    Ok(PositionInfo {
        position_type: PositionType::try_from(account.data[POSITION_OFFSET_POSITION_TYPE])
            .map_err(|error| ChainError::InvalidPositionType(error.0))?,
        status: PositionStatus::try_from(account.data[POSITION_OFFSET_STATUS])?,
        taker_owner: read_pubkey(&account.data, POSITION_OFFSET_TAKER_OWNER)?,
        maker_owner: read_pubkey(&account.data, POSITION_OFFSET_MAKER_OWNER)?,
        market_pda: read_pubkey(&account.data, POSITION_OFFSET_MARKET)?,
        strike: Strike::new(read_u64(&account.data, POSITION_OFFSET_STRIKE)?),
        quantity: Quantity::new(read_u64(&account.data, POSITION_OFFSET_QUANTITY)?),
        order_id: OrderId::new(order_id),
    })
}

const MARKET_OFFSET_UNDERLYING_DECIMALS: usize = 3;
const MARKET_OFFSET_QUOTE_DECIMALS: usize = 4;
const MARKET_OFFSET_UNDERLYING_MINT: usize = 32;
const MARKET_OFFSET_QUOTE_MINT: usize = 64;
const MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM: usize = 96;
const MARKET_OFFSET_QUOTE_TOKEN_PROGRAM: usize = 128;
const MARKET_MIN_LEN: usize = MARKET_OFFSET_QUOTE_TOKEN_PROGRAM + 32;

pub(crate) fn parse_market(
    account: &Account,
    program_id: &Pubkey,
) -> Result<MarketInfo, ChainError> {
    validate_account(account, program_id, MARKET_DISCRIMINATOR)?;
    if account.data.len() < MARKET_MIN_LEN {
        return Err(ChainError::InvalidAccountData);
    }
    Ok(MarketInfo {
        underlying_decimals: Decimals::new(account.data[MARKET_OFFSET_UNDERLYING_DECIMALS]),
        quote_decimals: Decimals::new(account.data[MARKET_OFFSET_QUOTE_DECIMALS]),
        underlying_mint: read_pubkey(&account.data, MARKET_OFFSET_UNDERLYING_MINT)?,
        quote_mint: read_pubkey(&account.data, MARKET_OFFSET_QUOTE_MINT)?,
        underlying_token_program_id: read_pubkey(
            &account.data,
            MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM,
        )?,
        quote_token_program_id: read_pubkey(&account.data, MARKET_OFFSET_QUOTE_TOKEN_PROGRAM)?,
    })
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
    if account
        .data
        .get(ACCOUNT_OFFSET_VERSION)
        .copied()
        .unwrap_or(0)
        == 0
    {
        return Err(ChainError::UninitializedAccount);
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
