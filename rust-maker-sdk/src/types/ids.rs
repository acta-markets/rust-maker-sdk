use std::fmt::Display;

use derive_more::{Display as DeriveDisplay, From, Into};
use serde::{Deserialize, Serialize};

define_bytes32_newtype!(OrderId);

define_numeric_newtype!(Strike, u64);
define_numeric_newtype!(Price, u64);
define_numeric_newtype!(Quantity, u64);
define_numeric_newtype!(Nonce, u64);
define_numeric_newtype!(RfqVersion, u64);
define_numeric_newtype!(OrderVersion, u64);
define_numeric_newtype!(Slot, u64);
define_numeric_newtype!(ChainId, u64);
define_numeric_newtype!(DurationSeconds, u64);
define_numeric_newtype!(Volume, u64);
define_numeric_newtype!(Balance, u64);

define_numeric_newtype!(QuoteCount, u32);
define_numeric_newtype!(TradeCount, u32);
define_numeric_newtype!(TimeoutSeconds, u32);

#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    From,
    Into,
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
