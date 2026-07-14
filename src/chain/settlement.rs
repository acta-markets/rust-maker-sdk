use crate::{PRICE_SCALE, PositionType};

use super::ChainError;

pub(super) fn required_settlement(
    position_type: u8,
    strike: u64,
    quantity: u64,
    quote_decimals: u8,
    underlying_decimals: u8,
) -> Result<u64, ChainError> {
    match PositionType::try_from(position_type) {
        Ok(PositionType::CoveredCall) => {
            scaled_quote_amount(strike, quantity, quote_decimals, underlying_decimals)
        }
        Ok(PositionType::CashSecuredPut) => Ok(quantity),
        Err(_) => Err(ChainError::InvalidPositionType(position_type)),
    }
}

fn scaled_quote_amount(
    strike: u64,
    quantity: u64,
    quote_decimals: u8,
    underlying_decimals: u8,
) -> Result<u64, ChainError> {
    let quote_scale = 10u128
        .checked_pow(u32::from(quote_decimals))
        .ok_or(ChainError::SettlementAmountOverflow)?;
    let underlying_scale = 10u128
        .checked_pow(u32::from(underlying_decimals))
        .ok_or(ChainError::SettlementAmountOverflow)?;
    let numerator = u128::from(strike)
        .checked_mul(u128::from(quantity))
        .and_then(|value| value.checked_mul(quote_scale))
        .ok_or(ChainError::SettlementAmountOverflow)?;
    let denominator = u128::from(PRICE_SCALE)
        .checked_mul(underlying_scale)
        .ok_or(ChainError::SettlementAmountOverflow)?;
    let amount = numerator / denominator;
    u64::try_from(amount).map_err(|_| ChainError::SettlementAmountOverflow)
}

#[cfg(test)]
#[path = "settlement_tests.rs"]
mod tests;
