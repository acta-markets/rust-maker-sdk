//! Exact `rust_decimal` views of 1e9-scaled values and atom amounts.

use rust_decimal::Decimal;

use super::PRICE_SCALE;
use super::ids::{Decimals, Price, Quantity, Strike};

const PRICE_DECIMALS: u32 = 9;

fn scaled(value: u64) -> Decimal {
    Decimal::from_i128_with_scale(i128::from(value), PRICE_DECIMALS)
}

/// `value × multiplier`, rounded down; `None` for a negative or out-of-range result.
fn floor_atoms(value: Decimal, multiplier: Decimal) -> Option<u64> {
    let atoms = value.checked_mul(multiplier)?.floor();
    u64::try_from(atoms).ok()
}

impl Strike {
    #[must_use]
    pub fn to_decimal(self) -> Decimal {
        scaled(self.value())
    }
}

impl Price {
    #[must_use]
    pub fn to_decimal(self) -> Decimal {
        scaled(self.value())
    }

    /// Quote tokens per underlying token, rounded down to the 1e-9 grid.
    #[must_use]
    pub fn from_decimal_floor(value: Decimal) -> Option<Self> {
        floor_atoms(value, Decimal::from(PRICE_SCALE)).map(Self::new)
    }
}

impl Quantity {
    /// `None` when `decimals` exceeds 28, the most `Decimal` represents.
    #[must_use]
    pub fn to_decimal(self, decimals: Decimals) -> Option<Decimal> {
        let scale = u32::from(decimals.value());
        (scale <= Decimal::MAX_SCALE)
            .then(|| Decimal::from_i128_with_scale(i128::from(self.value()), scale))
    }

    /// Whole underlying tokens to atoms, rounded down.
    #[must_use]
    pub fn from_decimal_floor(units: Decimal, decimals: Decimals) -> Option<Self> {
        let power = 10i128.checked_pow(u32::from(decimals.value()))?;
        let multiplier = Decimal::try_from_i128_with_scale(power, 0).ok()?;
        floor_atoms(units, multiplier).map(Self::new)
    }
}

#[cfg(test)]
#[path = "decimal_tests.rs"]
mod tests;
