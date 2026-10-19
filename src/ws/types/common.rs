use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use strum::IntoStaticStr;

use crate::types::ids::{
    ChainId, Decimals, MarketId, PositionType, Price, Quantity, Strike, TradeCount, Volume,
};

pub const FEATURE_QUOTE_EXPIRED: &str = "quote_expired";
/// The engine cancels the session's open quotes when it disconnects.
pub const FEATURE_CANCEL_ON_DISCONNECT: &str = "cancel_on_disconnect";

/// Code and reason the server puts on the WebSocket close frame it initiates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum CloseCode {
    IdleTimeout,
    AuthTimeout,
    SlowConsumer,
    RateLimited,
    SessionReplaced,
    HelloTimeout,
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
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum WsChannel {
    Stats,
    Rfqs,
    Trades,
    ChainEvents,
    Markets,
    Vaults,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum WsEndpointKind {
    Maker,
    MakerData,
    Taker,
    Vault,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum PositionUpdateType {
    Created,
    Funded,
    Liquidated,
    Settled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum PositionStatus {
    None,
    Open,
    Funded,
    Liquidated,
    Settled,
    #[serde(other)]
    Unknown,
}

/// Reconciliation detail reported alongside the public position status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum PositionReconciliationStatus {
    OrphanPending,
    OrphanedClosed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqOrderOption {
    pub strike: Strike,
}

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
    pub program_id: String,
    pub market_pda: String,
    pub underlying_mint: String,
    pub quote_mint: String,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub collateral_mint: String,
    pub settlement_mint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDescriptorInfo {
    pub market: MarketDescriptor,
    pub underlying_oracle_pda: String,
    pub quote_oracle_pda: String,
    pub underlying_decimals: Decimals,
    pub quote_decimals: Decimals,
    pub size_rule: PositionSizeRule,
    pub underlying_symbol: String,
    pub quote_symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenInfo {
    pub mint: String,
    pub symbol: String,
    pub decimals: Decimals,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oracle_feed_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_rule: Option<PositionSizeRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike_tick: Option<Strike>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quote_mints: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usd: Option<UsdTradeStats>,
    pub total_volume_24h: Volume,
    pub total_trades_24h: TradeCount,
    pub total_price_24h: Volume,
    pub active_markets: u32,
    pub active_makers: u32,
    pub active_rfqs: u32,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionInfo {
    pub pda: String,
    pub market: MarketId,
    pub underlying_mint: String,
    pub quote_mint: String,
    pub position_type: PositionType,
    pub status: PositionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciliation_status: Option<PositionReconciliationStatus>,
    pub strike: Strike,
    pub quantity: Quantity,
    pub price: Price,
    #[serde(default)]
    pub total_premium: Price,
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_otm: Option<bool>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeInfo {
    pub id: uuid::Uuid,
    pub market: MarketId,
    pub position_type: PositionType,
    pub strike: Strike,
    pub quantity: Quantity,
    pub price: Price,
    pub taker: String,
    pub maker: String,
    pub tx_signature: String,
    #[serde_as(as = "UnixSeconds")]
    pub executed_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_added: Option<Volume>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trades_added: Option<TradeCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_added: Option<Volume>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketStats {
    pub volume_24h: Volume,
    pub trades_24h: TradeCount,
}

#[cfg(test)]
mod usd_stats_tests {
    use super::*;

    #[test]
    fn old_snapshots_remain_readable_and_usd_amounts_stay_exact() {
        let old = serde_json::to_value(GlobalStats::default()).unwrap();
        assert!(old.get("usd").is_none());
        assert!(
            serde_json::from_value::<GlobalStats>(old)
                .unwrap()
                .usd
                .is_none()
        );
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
