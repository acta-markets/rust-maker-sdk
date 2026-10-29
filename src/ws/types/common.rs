use crate::Address32;
use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use strum::IntoStaticStr;

use crate::types::TakerSide;
use crate::types::ids::{
    ChainId, Decimals, MarketId, OptionTerms, Price, Quantity, Strike, TradeCount, Volume,
};

/// Code and reason the server puts on the WebSocket close frame it initiates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum CloseCode {
    IdleTimeout,
    AuthTimeout,
    SlowConsumer,
    RateLimited,
    SessionReplaced,
    HelloTimeout,
    UnsupportedProtocolVersion,
}

impl CloseCode {
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::IdleTimeout => 4000,
            Self::AuthTimeout => 4001,
            Self::SlowConsumer => 4002,
            Self::RateLimited => 4003,
            Self::SessionReplaced => 4004,
            Self::HelloTimeout => 4005,
            Self::UnsupportedProtocolVersion => 4006,
        }
    }

    #[must_use]
    pub fn reason(self) -> &'static str {
        self.into()
    }

    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            4000 => Some(Self::IdleTimeout),
            4001 => Some(Self::AuthTimeout),
            4002 => Some(Self::SlowConsumer),
            4003 => Some(Self::RateLimited),
            4004 => Some(Self::SessionReplaced),
            4005 => Some(Self::HelloTimeout),
            4006 => Some(Self::UnsupportedProtocolVersion),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WsChannel {
    Stats,
    Rfqs,
    Trades,
    ChainEvents,
    Markets,
    Vaults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WsEndpointKind {
    Maker,
    MakerData,
    Taker,
    Vault,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PositionUpdateType {
    Created,
    Funded,
    Liquidated,
    Settled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PositionStatus {
    Open,
    Funded,
    Liquidated,
    Settled,
}

/// A selectable RFQ term set. Physical RFQs may advertise a ladder; a spread
/// contains exactly its requested term set.
pub type RfqOrderOption = OptionTerms;

/// Position size constraints expressed in **underlying** token units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionSizeRule {
    pub min_size: Quantity,
    pub max_size: Quantity,
    pub step: Quantity,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDescriptor {
    pub chain_id: ChainId,
    pub program_id: Address32,
    pub market_pda: Address32,
    pub underlying_mint: Address32,
    pub quote_mint: Address32,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub collateral_mint: Address32,
    pub settlement_mint: Address32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDescriptorInfo {
    pub products: crate::types::ProductSet,
    pub market: MarketDescriptor,
    pub underlying_oracle_pda: Address32,
    pub quote_oracle_pda: Address32,
    pub underlying_decimals: Decimals,
    pub quote_decimals: Decimals,
    pub size_rule: PositionSizeRule,
    pub underlying_symbol: String,
    pub quote_symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenInfo {
    pub mint: Address32,
    pub symbol: String,
    pub decimals: Decimals,
    pub oracle_feed_ids: Vec<String>,
    pub role: TokenRole,
}

/// Only underlyings have option sizing and a strike grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TokenRole {
    Quote {},
    Underlying {
        size_rule: PositionSizeRule,
        strike_tick: Strike,
        quote_mints: Vec<Address32>,
    },
}

/// USD totals from prices captured at signed-transaction submission.
/// Amounts are decimal dollar strings; priced count exposes historical gaps.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsdTradeStats {
    pub notional_24h: String,
    pub premium_24h: String,
    pub priced_trades_24h: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalStats {
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub usd: Option<UsdTradeStats>,
    pub total_volume_24h: Volume,
    pub total_trades_24h: TradeCount,
    /// Cash-settled spread fills, excluded from physical aggregate volume and trade totals.
    pub spread_trades_24h: TradeCount,
    pub total_price_24h: Volume,
    pub active_markets: u32,
    pub active_makers: u32,
    pub active_rfqs: u32,
}

/// Settlement facts: cash amounts are quote atoms; open positions have no settlement facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PositionSettlement {
    Physical {
        settlement_price: Price,
        is_otm: bool,
    },
    Cash {
        settlement_price: Price,
        maker_payoff: crate::types::Balance,
        taker_return: crate::types::Balance,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PositionState {
    Open {},
    Funded {},
    Liquidated {},
    Settled { settlement: PositionSettlement },
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionInfo {
    pub revision: crate::types::ids::PositionRevision,
    pub pda: Address32,
    pub market: MarketId,
    pub underlying_mint: Address32,
    pub underlying_decimals: u8,
    pub quote_mint: Address32,
    pub quote_decimals: u8,
    pub terms: OptionTerms,
    pub side: TakerSide,
    #[serde(flatten)]
    pub state: PositionState,
    pub quantity: Quantity,
    pub price: Price,
    /// Net premium in quote atoms, after the protocol fee.
    pub total_premium: crate::types::ids::Balance,
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeInfo {
    pub id: uuid::Uuid,
    pub market: MarketId,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    pub price: Price,
    pub taker: Address32,
    pub maker: Address32,
    pub tx_signature: String,
    #[serde_as(as = "UnixSeconds")]
    pub executed_at: SystemTime,
}

#[cfg(test)]
mod usd_stats_tests {
    use super::*;

    #[test]
    fn usd_stats_are_required_nullable_and_stay_exact() {
        let unavailable = serde_json::to_value(GlobalStats::default()).unwrap();
        assert_eq!(unavailable.get("usd"), Some(&serde_json::Value::Null));
        let mut omitted = unavailable;
        let _ = omitted.as_object_mut().unwrap().remove("usd");
        assert!(serde_json::from_value::<GlobalStats>(omitted).is_err());
        let stats = GlobalStats {
            usd: Some(UsdTradeStats {
                notional_24h: "9007199254740993.000000001".into(),
                premium_24h: "0.000000009".into(),
                priced_trades_24h: 2,
            }),
            ..GlobalStats::default()
        };
        let decoded: GlobalStats =
            serde_json::from_str(&serde_json::to_string(&stats).unwrap()).unwrap();
        assert_eq!(decoded.usd, stats.usd);
    }
}

#[cfg(test)]
mod close_code_tests {
    use super::CloseCode;

    #[test]
    fn close_codes_round_trip_through_the_wire_number() {
        for (close, code, reason) in [
            (CloseCode::IdleTimeout, 4000, "idle_timeout"),
            (CloseCode::AuthTimeout, 4001, "auth_timeout"),
            (CloseCode::SlowConsumer, 4002, "slow_consumer"),
            (CloseCode::RateLimited, 4003, "rate_limited"),
            (CloseCode::SessionReplaced, 4004, "session_replaced"),
            (CloseCode::HelloTimeout, 4005, "hello_timeout"),
        ] {
            assert_eq!((close.code(), close.reason()), (code, reason));
            assert_eq!(CloseCode::from_code(code), Some(close));
        }
        assert_eq!(CloseCode::from_code(1000), None);
    }
}

/// Quote handling when the authenticated connection closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnDisconnect {
    CancelQuotes,
    KeepQuotes,
}

/// A single-value filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueryFilter<T> {
    All {},
    Only { value: T },
}

impl<T> QueryFilter<T> {
    pub fn only(self) -> Option<T> {
        match self {
            Self::All {} => None,
            Self::Only { value } => Some(value),
        }
    }
}

/// An optional time bound on a range query.
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimeBound {
    Unbounded {},
    Bounded {
        #[serde_as(as = "UnixSeconds")]
        value: SystemTime,
    },
}

impl TimeBound {
    pub fn bound(self) -> Option<SystemTime> {
        match self {
            Self::Unbounded {} => None,
            Self::Bounded { value } => Some(value),
        }
    }
}

/// Keyset page position: `at` is the ordering timestamp of the last row seen.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageCursor<I> {
    Start {},
    After {
        #[serde_as(as = "UnixSeconds")]
        at: SystemTime,
        id: I,
    },
}
