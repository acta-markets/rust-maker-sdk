use crate::PositionType;
use crate::types::amounts::scaled_quote_amount;

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
                .ok_or(ChainError::SettlementAmountOverflow)
        }
        Ok(PositionType::CashSecuredPut) => Ok(quantity),
        Err(_) => Err(ChainError::InvalidPositionType(position_type)),
    }
}

#[cfg(test)]
#[path = "settlement_tests.rs"]
mod tests;
