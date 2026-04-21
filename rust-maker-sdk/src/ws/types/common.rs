use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};
use strum::IntoStaticStr;

use crate::types::ids::{
    ChainId, Decimals, MarketId, PositionType, Price, Quantity, Strike, TradeCount, Volume,
};

pub use crate::types::AuthRequiredAction;
pub use crate::types::CapError;
pub use crate::types::DbFeature;
pub use crate::types::MakerBalanceCapInfo;
pub use crate::types::MakerNotionalCapInfo;
pub use crate::types::MakerPositionCapInfo;
pub use crate::types::MarketCapInfo;
pub use crate::types::QuoteCancelReason;
pub use crate::types::QuoteCapInfo;
pub use crate::types::QuoteFinalStatus;
pub use crate::types::QuoteLockedReason;
pub use crate::types::QuoteStatus;
pub use crate::types::RateLimitReason;
pub use crate::types::RfqAvailableAgainReason;
pub use crate::types::RfqCloseReason;
pub use crate::types::RfqStateError;
pub use crate::types::TokenCapInfo;
pub use crate::types::UserRole;

pub const FEATURE_QUOTE_EXPIRED: &str = "quote_expired";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WsChannel {
    Stats,
    Rfqs,
    Trades,
    Positions,
    ChainEvents,
    Markets,
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
    None,
    Open,
    Funded,
    Liquidated,
    Settled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqOrderOption {
    pub strike: Strike,
}

/// Position size constraints expressed in **underlying** token units.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde_as(as = "TimestampSeconds<i64>")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub mint: String,
    pub decimals: Decimals,
    pub size_rule: PositionSizeRule,
    pub symbol: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalStats {
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
    pub status: String,
    pub strike: Strike,
    pub quantity: Quantity,
    pub price: Price,
    #[serde(default)]
    pub total_premium: Price,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub created_at: SystemTime,
    #[serde_as(as = "TimestampSeconds<i64>")]
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
    #[serde_as(as = "TimestampSeconds<i64>")]
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

#[must_use]
pub const fn default_true() -> bool {
    true
}
