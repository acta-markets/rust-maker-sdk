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
    /// Subtract a floor-rounded per-unit fee, saturating at zero.
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
    /// The server has no authoritative record for the requested order ID.
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

/// An address-valued identity with a cached canonical representation.
macro_rules! define_address_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(from = "crate::Address32", into = "crate::Address32")]
        pub struct $name {
            address: crate::Address32,
            base58: String,
        }
        impl $name {
            #[must_use]
            pub fn new(address: crate::Address32) -> Self {
                Self {
                    address,
                    base58: address.to_base58(),
                }
            }
            #[must_use]
            pub const fn address(&self) -> crate::Address32 {
                self.address
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.base58
            }
        }
        impl From<crate::Address32> for $name {
            fn from(value: crate::Address32) -> Self {
                Self::new(value)
            }
        }
        impl From<$name> for crate::Address32 {
            fn from(value: $name) -> Self {
                value.address
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }
    };
}

define_address_id!(MarketId);
define_address_id!(UserId);

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
pub enum PositionType {
    CoveredCall,
    CashSecuredPut,
    CallSpread,
    PutSpread,
    CashPut,
}

impl Display for PositionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.into())
    }
}

impl PositionType {
    pub const fn is_spread(self) -> bool {
        matches!(self, Self::CallSpread | Self::PutSpread | Self::CashPut)
    }
    pub const fn is_put(self) -> bool {
        matches!(self, Self::CashSecuredPut | Self::PutSpread | Self::CashPut)
    }
}

/// The complete, hash-bound option terms carried by an RFQ, quote, position, or trade.
///
/// The tagged form prevents a physical option from carrying a second strike and
/// prevents a spread from losing either strike during a wire round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OptionTerms {
    /// Cash-settled put: wire strike K, execution legs (0, K).
    CashPut {
        strike: Strike,
    },
    CoveredCall {
        strike: Strike,
    },
    CashSecuredPut {
        strike: Strike,
    },
    CallSpread {
        lower_strike: Strike,
        upper_strike: Strike,
    },
    PutSpread {
        lower_strike: Strike,
        upper_strike: Strike,
    },
}

impl OptionTerms {
    pub const fn is_spread(self) -> bool {
        self.position_type().is_spread()
    }
    /// Replace the physical strike (or spread lower strike), retaining the product and K2.
    #[must_use]
    pub const fn with_strike(self, strike: Strike) -> Self {
        match self {
            Self::CashPut { .. } => Self::CashPut { strike },
            Self::CoveredCall { .. } => Self::CoveredCall { strike },
            Self::CashSecuredPut { .. } => Self::CashSecuredPut { strike },
            Self::CallSpread { upper_strike, .. } => Self::CallSpread {
                lower_strike: strike,
                upper_strike,
            },
            Self::PutSpread { upper_strike, .. } => Self::PutSpread {
                lower_strike: strike,
                upper_strike,
            },
        }
    }

    #[must_use]
    pub const fn position_type(self) -> PositionType {
        match self {
            Self::CashPut { .. } => PositionType::CashPut,
            Self::CoveredCall { .. } => PositionType::CoveredCall,
            Self::CashSecuredPut { .. } => PositionType::CashSecuredPut,
            Self::CallSpread { .. } => PositionType::CallSpread,
            Self::PutSpread { .. } => PositionType::PutSpread,
        }
    }

    #[must_use]
    pub const fn strike(self) -> Strike {
        match self {
            Self::CashPut { strike } => strike,
            Self::CoveredCall { strike } | Self::CashSecuredPut { strike } => strike,
            Self::CallSpread { lower_strike, .. } | Self::PutSpread { lower_strike, .. } => {
                lower_strike
            }
        }
    }

    #[must_use]
    pub const fn upper_strike(self) -> Option<Strike> {
        match self {
            Self::CashPut { strike } => Some(strike),
            Self::CoveredCall { .. } | Self::CashSecuredPut { .. } => None,
            Self::CallSpread { upper_strike, .. } | Self::PutSpread { upper_strike, .. } => {
                Some(upper_strike)
            }
        }
    }

    /// Convert public economic terms to the contract's four-product representation.
    #[must_use]
    pub const fn to_chain(self) -> ChainTerms {
        match self {
            Self::CoveredCall { strike } => ChainTerms::CoveredCall { strike },
            Self::CashSecuredPut { strike } => ChainTerms::CashSecuredPut { strike },
            Self::CallSpread {
                lower_strike,
                upper_strike,
            } => ChainTerms::CallSpread {
                lower_strike,
                upper_strike,
            },
            Self::PutSpread {
                lower_strike,
                upper_strike,
            } => ChainTerms::PutSpread {
                lower_strike,
                upper_strike,
            },
            Self::CashPut { strike } => ChainTerms::PutSpread {
                lower_strike: Strike(0),
                upper_strike: strike,
            },
        }
    }

    /// The sole normalization of a zero-lower chain put spread into a cash put.
    #[must_use]
    pub const fn from_chain(terms: ChainTerms) -> Self {
        match terms {
            ChainTerms::CoveredCall { strike } => Self::CoveredCall { strike },
            ChainTerms::CashSecuredPut { strike } => Self::CashSecuredPut { strike },
            ChainTerms::CallSpread {
                lower_strike,
                upper_strike,
            } => Self::CallSpread {
                lower_strike,
                upper_strike,
            },
            ChainTerms::PutSpread {
                lower_strike: Strike(0),
                upper_strike,
            } => Self::CashPut {
                strike: upper_strike,
            },
            ChainTerms::PutSpread {
                lower_strike,
                upper_strike,
            } => Self::PutSpread {
                lower_strike,
                upper_strike,
            },
        }
    }

    pub fn from_chain_parts(
        product: ChainPositionType,
        strike: Strike,
        upper: Option<Strike>,
    ) -> Result<Self, crate::spreads::SpreadError> {
        ChainTerms::from_parts(product, strike, upper).map(Self::from_chain)
    }
}

/// Contract discriminants only. Public products have no numeric conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ChainPositionType {
    CoveredCall = 0,
    CashSecuredPut = 1,
    CallSpread = 2,
    PutSpread = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid chain position type value: {0}")]
pub struct ChainPositionTypeParseError(pub u8);

impl TryFrom<u8> for ChainPositionType {
    type Error = ChainPositionTypeParseError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::CoveredCall),
            1 => Ok(Self::CashSecuredPut),
            2 => Ok(Self::CallSpread),
            3 => Ok(Self::PutSpread),
            _ => Err(ChainPositionTypeParseError(value)),
        }
    }
}
impl From<ChainPositionType> for u8 {
    fn from(value: ChainPositionType) -> Self {
        value as Self
    }
}
impl ChainPositionType {
    #[must_use]
    pub const fn is_spread(self) -> bool {
        matches!(self, Self::CallSpread | Self::PutSpread)
    }
    #[must_use]
    pub const fn is_put(self) -> bool {
        matches!(self, Self::CashSecuredPut | Self::PutSpread)
    }
    #[must_use]
    pub const fn uses_quote_collateral(self) -> bool {
        !matches!(self, Self::CoveredCall)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainTerms {
    CoveredCall {
        strike: Strike,
    },
    CashSecuredPut {
        strike: Strike,
    },
    CallSpread {
        lower_strike: Strike,
        upper_strike: Strike,
    },
    PutSpread {
        lower_strike: Strike,
        upper_strike: Strike,
    },
}
impl ChainTerms {
    pub fn from_parts(
        product: ChainPositionType,
        strike: Strike,
        upper: Option<Strike>,
    ) -> Result<Self, crate::spreads::SpreadError> {
        match (product, upper) {
            (ChainPositionType::CoveredCall, None) if strike.0 > 0 => {
                Ok(Self::CoveredCall { strike })
            }
            (ChainPositionType::CashSecuredPut, None) if strike.0 > 0 => {
                Ok(Self::CashSecuredPut { strike })
            }
            (ChainPositionType::CallSpread, Some(upper_strike))
                if strike.0 > 0 && upper_strike > strike =>
            {
                Ok(Self::CallSpread {
                    lower_strike: strike,
                    upper_strike,
                })
            }
            (ChainPositionType::PutSpread, Some(upper_strike)) if upper_strike > strike => {
                Ok(Self::PutSpread {
                    lower_strike: strike,
                    upper_strike,
                })
            }
            _ => Err(crate::spreads::SpreadError::InvalidStrikes),
        }
    }
    #[must_use]
    pub const fn position_type(self) -> ChainPositionType {
        match self {
            Self::CoveredCall { .. } => ChainPositionType::CoveredCall,
            Self::CashSecuredPut { .. } => ChainPositionType::CashSecuredPut,
            Self::CallSpread { .. } => ChainPositionType::CallSpread,
            Self::PutSpread { .. } => ChainPositionType::PutSpread,
        }
    }
    #[must_use]
    pub const fn strike(self) -> Strike {
        match self {
            Self::CoveredCall { strike } | Self::CashSecuredPut { strike } => strike,
            Self::CallSpread { lower_strike, .. } | Self::PutSpread { lower_strike, .. } => {
                lower_strike
            }
        }
    }
    #[must_use]
    pub const fn upper_strike(self) -> Option<Strike> {
        match self {
            Self::CoveredCall { .. } | Self::CashSecuredPut { .. } => None,
            Self::CallSpread { upper_strike, .. } | Self::PutSpread { upper_strike, .. } => {
                Some(upper_strike)
            }
        }
    }
}

/// Taker direction committed in the signed order preimage.
///
/// The venue currently admits only [`Self::Sell`]. `Buy` remains representable
/// so the wire and preimage layout do not need another migration when economics
/// for taker buys are added.
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
pub enum TakerSide {
    Sell,
    Buy,
}

impl TakerSide {
    #[must_use]
    pub const fn is_taker_buy(self) -> bool {
        matches!(self, Self::Buy)
    }
}
/// Last applied lifecycle event in canonical chain order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct PositionRevision {
    pub slot: u64,
    pub transaction_index: u32,
    pub instruction_index: u16,
}
