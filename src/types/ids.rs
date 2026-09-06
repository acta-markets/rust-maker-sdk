use std::fmt::Display;

use serde::{Deserialize, Serialize};

define_bytes32_newtype!(OrderId);

define_numeric_newtype!(
    /// Strike price, 1e9 fixed-point per one unit of the underlying
    /// ([`PRICE_SCALE`](crate::PRICE_SCALE)), independent of mint decimals.
    Strike,
    u64
);
define_numeric_newtype!(
    /// Gross premium per one unit of the underlying, 1e9 fixed-point
    /// ([`PRICE_SCALE`](crate::PRICE_SCALE)), independent of mint decimals.
    Price,
    u64
);
define_numeric_newtype!(
    /// Order size in the underlying mint's atomic units.
    Quantity,
    u64
);

impl Price {
    /// Apply a protocol fee in basis points using the contract's rounding rule.
    #[must_use]
    pub fn after_fee_bps(self, bps: u16) -> Self {
        let fee = u128::from(self.0) * u128::from(bps) / 10_000;
        Self::new(
            self.0
                .saturating_sub(u64::try_from(fee).unwrap_or(u64::MAX)),
        )
    }
}

define_numeric_newtype!(Nonce, u64);
define_numeric_newtype!(RfqVersion, u64);
define_numeric_newtype!(OrderVersion, u64);

impl OrderVersion {
    /// Order exists and has been accepted/locked by the taker.
    pub const ACCEPTED: Self = Self::new(1);
    /// The signed transaction has been submitted for execution.
    pub const SUBMITTED: Self = Self::new(2);
    /// A locally final failure or expiry. A later chain confirmation may supersede failure.
    pub const FAILED: Self = Self::new(3);
    /// An authoritative on-chain confirmation.
    pub const CONFIRMED: Self = Self::new(4);
    /// No authoritative order exists for the requested id.
    pub const NOT_FOUND: Self = Self::new(0);
}
define_numeric_newtype!(Slot, u64);
define_numeric_newtype!(ChainId, u64);
define_numeric_newtype!(DurationSeconds, u64);
define_numeric_newtype!(Volume, u64);
define_numeric_newtype!(Balance, u64);

define_numeric_newtype!(QuoteCount, u32);
define_numeric_newtype!(TradeCount, u32);
define_numeric_newtype!(TimeoutSeconds, u32);

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Decimals(pub u8);

impl Decimals {
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl Display for Decimals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u8> for Decimals {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<Decimals> for u8 {
    fn from(value: Decimals) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::OrderVersion;

    #[test]
    fn order_version_ranks_match_wire_lifecycle_precedence() {
        assert_eq!(OrderVersion::NOT_FOUND.value(), 0);
        assert!(OrderVersion::ACCEPTED < OrderVersion::SUBMITTED);
        assert!(OrderVersion::SUBMITTED < OrderVersion::FAILED);
        assert!(OrderVersion::FAILED < OrderVersion::CONFIRMED);
    }
}

define_string_newtype!(MarketId);
define_string_newtype!(UserId);

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::IntoStaticStr,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[repr(u8)]
pub enum PositionType {
    CoveredCall = 0,
    CashSecuredPut = 1,
}

impl From<PositionType> for u8 {
    fn from(position_type: PositionType) -> Self {
        position_type as Self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid position type value: {0}")]
pub struct PositionTypeParseError(pub u8);

impl TryFrom<u8> for PositionType {
    type Error = PositionTypeParseError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::CoveredCall),
            1 => Ok(Self::CashSecuredPut),
            _ => Err(PositionTypeParseError(value)),
        }
    }
}

impl Display for PositionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.into())
    }
}
