//! Human-unit views and contract amount math for 1e9-scaled values.

use super::PRICE_SCALE;
use super::ids::{Decimals, Price, Quantity, Strike};

#[allow(clippy::cast_precision_loss)]
fn scaled_to_f64(value: u64) -> f64 {
    value as f64 / PRICE_SCALE as f64
}

impl Strike {
    /// Quote tokens per one underlying token, for display.
    #[must_use]
    pub fn as_decimal(self) -> f64 {
        scaled_to_f64(self.value())
    }
}

impl Price {
    /// Premium in quote tokens per one underlying token, for display.
    #[must_use]
    pub fn as_decimal(self) -> f64 {
        scaled_to_f64(self.value())
    }
}

impl Quantity {
    /// Underlying atoms as whole tokens, for display.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn to_units(self, underlying_decimals: Decimals) -> f64 {
        self.value() as f64 / 10f64.powi(i32::from(underlying_decimals.value()))
    }
}

/// Gross premium of a fill before protocol fees, in quote-mint atoms, rounded
/// down exactly as the contract computes it. `None` when decimals exceed 18 or the
/// result does not fit `u64`.
#[must_use]
pub fn total_premium_quote_atoms(
    price: Price,
    quantity: Quantity,
    underlying_decimals: Decimals,
    quote_decimals: Decimals,
) -> Option<u64> {
    scaled_quote_amount(
        price.value(),
        quantity.value(),
        quote_decimals.value(),
        underlying_decimals.value(),
    )
}

/// A covered-call or cash-secured-put fill's premium split, in quote atoms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalPremium {
    pub gross_premium: u64,
    pub protocol_fee: u64,
    pub net_premium: u64,
}

/// The contract's fee: the smaller of the premium fee and the volume fee on the
/// strike notional. `None` as for [`total_premium_quote_atoms`] or when a rate
/// exceeds 10 000 bps. Rates come from `/api/v1/venue`.
#[must_use]
pub fn physical_premium(
    price: Price,
    strike: Strike,
    quantity: Quantity,
    underlying_decimals: Decimals,
    quote_decimals: Decimals,
    premium_bps: u16,
    volume_bps: u16,
) -> Option<PhysicalPremium> {
    if premium_bps > 10_000 || volume_bps > 10_000 {
        return None;
    }
    let quote = |per_unit: u64| {
        scaled_quote_amount(
            per_unit,
            quantity.value(),
            quote_decimals.value(),
            underlying_decimals.value(),
        )
    };
    let fee =
        |amount: u64, bps: u16| u64::try_from(u128::from(amount) * u128::from(bps) / 10_000).ok();
    let gross_premium = quote(price.value())?;
    let premium_fee = gross_premium - quote(price.value() - fee(price.value(), premium_bps)?)?;
    let protocol_fee = premium_fee.min(fee(quote(strike.value())?, volume_bps)?);
    Some(PhysicalPremium {
        gross_premium,
        protocol_fee,
        net_premium: gross_premium - protocol_fee,
    })
}

/// The contract's `scaled_quote_amount`: `per_unit` (1e9-scaled quote per
/// underlying token) times `quantity` (underlying atoms), in quote atoms.
pub(crate) fn scaled_quote_amount(
    per_unit: u64,
    quantity: u64,
    quote_decimals: u8,
    underlying_decimals: u8,
) -> Option<u64> {
    if quote_decimals > 18 || underlying_decimals > 18 {
        return None;
    }
    let quote_scale = 10u128.pow(u32::from(quote_decimals));
    let denominator = u128::from(PRICE_SCALE) * 10u128.pow(u32::from(underlying_decimals));
    let numerator = u128::from(per_unit) * u128::from(quantity);
    let amount = if quote_scale <= denominator {
        numerator / (denominator / quote_scale)
    } else {
        numerator.checked_mul(quote_scale / denominator)?
    };
    u64::try_from(amount).ok()
}

#[cfg(test)]
#[path = "amounts_tests.rs"]
mod tests;
