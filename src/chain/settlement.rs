use crate::ChainPositionType;
use crate::types::amounts::scaled_quote_amount;

use super::ChainError;

pub(super) fn required_settlement(
    position_type: u8,
    strike: u64,
    quantity: u64,
    quote_decimals: u8,
    underlying_decimals: u8,
) -> Result<u64, ChainError> {
    match ChainPositionType::try_from(position_type) {
        Ok(ChainPositionType::CoveredCall) => {
            scaled_quote_amount(strike, quantity, quote_decimals, underlying_decimals)
                .ok_or(ChainError::SettlementAmountOverflow)
        }
        Ok(ChainPositionType::CashSecuredPut) => Ok(quantity),
        Ok(ChainPositionType::CallSpread | ChainPositionType::PutSpread) | Err(_) => {
            Err(ChainError::InvalidPositionType(position_type))
        }
    }
}

#[cfg(test)]
#[path = "settlement_tests.rs"]
mod tests;
