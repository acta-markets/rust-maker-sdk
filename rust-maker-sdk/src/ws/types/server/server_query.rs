use std::collections::HashMap;
use std::time::SystemTime;

use crate::types::ids::{
    Decimals, MarketId, OrderId, OrderVersion, PositionType, Price, Quantity, Slot, Strike,
};
use crate::types::{
    MakerBalanceCapInfo, MakerNotionalCapInfo, MakerPositionCapInfo, MarketCapInfo, TokenCapInfo,
};
use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};
use uuid::Uuid;

use super::super::common::{
    GlobalStats, MarketDescriptorInfo, MarketStats, PositionInfo, PositionSizeRule,
    PositionStatus, PositionUpdateType, QuoteStatus, StatsDelta, TokenInfo, TradeInfo, WsChannel,
};
use super::super::market::MarketInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionsData {
    pub request_id: Uuid,
    pub positions: Vec<PositionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketsData {
    pub request_id: Uuid,
    pub markets: Vec<MarketInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDescriptorsData {
    pub request_id: Uuid,
    pub markets: Vec<MarketDescriptorInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpiriesData {
    pub request_id: Uuid,
    #[serde_as(as = "Vec<TimestampSeconds<i64>>")]
    pub expiries_ts: Vec<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokensData {
    pub request_id: Uuid,
    pub underlyings: Vec<TokenInfo>,
    pub quotes_by_underlying: HashMap<String, Vec<TokenInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerPositionsMessage {
    pub request_id: Uuid,
    pub positions: Vec<MakerPositionInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerPositionInfo {
    pub pda: String,
    pub market: MarketId,
    pub underlying_mint: String,
    pub underlying_symbol: String,
    pub underlying_decimals: u8,
    pub quote_mint: String,
    pub quote_symbol: String,
    pub quote_decimals: u8,
    pub position_type: PositionType,
    pub status: PositionStatus,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyQuotesMessage {
    pub request_id: Uuid,
    pub quotes: Vec<MakerQuoteInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerQuoteInfo {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub market: MarketId,
    pub underlying_mint: String,
    pub underlying_symbol: String,
    pub underlying_decimals: u8,
    pub quote_mint: String,
    pub quote_symbol: String,
    pub quote_decimals: u8,
    pub strike: Strike,
    pub price: Price,
    pub quantity: Quantity,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub valid_until: SystemTime,
    pub status: QuoteStatus,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub created_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerMarketsMessage {
    pub request_id: Uuid,
    pub markets: Vec<MakerMarketInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerMarketInfo {
    pub market_pda: String,
    pub underlying_mint: String,
    pub quote_mint: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub is_finalized: bool,
    pub underlying_symbol: String,
    pub quote_symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<MarketStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyTradesMessage {
    pub request_id: Uuid,
    pub trades: Vec<MakerTradeInfo>,
    pub has_more: bool,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerTradeInfo {
    pub id: Uuid,
    pub rfq_id: Uuid,
    pub market_pda: String,
    pub underlying_mint: String,
    pub underlying_symbol: String,
    pub underlying_decimals: u8,
    pub quote_mint: String,
    pub quote_symbol: String,
    pub quote_decimals: u8,
    pub position_type: PositionType,
    pub taker: String,
    pub strike: Strike,
    pub quantity: Quantity,
    pub price: Price,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_pda: Option<String>,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub confirmed_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarnSummaryData {
    pub request_id: Uuid,
    pub assets: Vec<EarnAssetSummary>,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub computed_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarnAssetSummary {
    pub underlying_mint: String,
    pub underlying_symbol: String,
    pub quote_mint: String,
    pub quote_symbol: String,
    pub position_type: PositionType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_apr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_apr: Option<f64>,
    pub cap_filled_pct: f64,
    pub cap_total: Quantity,
    pub cap_used: Quantity,
    pub strikes_count: u32,
    pub nearest_market_pda: String,
    pub markets_count: u32,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub nearest_expiry_ts: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMarketsInfoData {
    pub request_id: Uuid,
    pub underlying_symbol: String,
    pub underlying_decimals: Decimals,
    pub quote_symbol: String,
    pub quote_decimals: Decimals,
    pub size_rule: PositionSizeRule,
    pub reference_price: Price,
    pub markets: Vec<TokenMarketEntry>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMarketEntry {
    pub market_pda: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub indicatives: Vec<TokenMarketIndicatives>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMarketIndicatives {
    pub position_type: PositionType,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub updated_at: SystemTime,
    pub is_stale: bool,
    pub strikes: Vec<IndicativeStrikeBest>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicativePricesMessage {
    pub request_id: Uuid,
    pub market: MarketId,
    pub position_type: PositionType,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub updated_at: SystemTime,
    pub is_stale: bool,
    pub strikes: Vec<IndicativeStrikeBest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicativeStrikeBest {
    pub strike: Strike,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCapsData {
    pub request_id: Uuid,
    pub tokens: Vec<TokenCapInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markets: Vec<MarketCapInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quotes: Vec<super::super::common::QuoteCapInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyCapsData {
    pub request_id: Uuid,
    pub positions: MakerPositionCapInfo,
    pub notional: Vec<MakerNotionalCapInfo>,
    pub balances: Vec<MakerBalanceCapInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerCapsSnapshot {
    pub positions: MakerPositionCapInfo,
    pub notional: Vec<MakerNotionalCapInfo>,
    pub balances: Vec<MakerBalanceCapInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmSummaryData {
    pub request_id: Uuid,
    pub maker_pda: String,
    pub caps: MyCapsData,
    pub positions: Vec<MakerPositionInfo>,
    pub active_quotes: Vec<MakerQuoteInfo>,
    pub markets: Vec<MakerMarketInfo>,
    pub tokens: Vec<TokenInfo>,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub computed_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionsMessage {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_mints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionUpdatedData {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_mints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderStatusMessage {
    pub request_id: Uuid,
    pub order_id: OrderId,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfq_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_pda: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMessage {
    pub markets: Vec<MarketInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeExecutedMessage {
    pub trade: TradeInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_delta: Option<StatsDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionUpdatedMessage {
    pub position: PositionInfo,
    pub update_type: PositionUpdateType,
    pub caps_snapshot: MakerCapsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUpdateMessage {
    pub stats: GlobalStats,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SponsoredTxToSignData {
    pub order_id: OrderId,
    pub tx_base64: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub signature_deadline: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderAcceptedData {
    pub order_id: OrderId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSubmittedData {
    pub order_id: OrderId,
    pub tx_signature: String,
    #[serde(default)]
    pub order_version: OrderVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderConfirmedData {
    pub order_id: OrderId,
    pub position_pda: String,
    #[serde(default)]
    pub order_version: OrderVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFailedData {
    pub order_id: OrderId,
    pub reason: String,
    #[serde(default)]
    pub order_version: OrderVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketFinalizedData {
    pub market_pda: String,
    pub settlement_price: Price,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum ChainEventMessage {
    PositionOpened(PositionOpenedEvent),
    MarketCreated(MarketCreatedEvent),
    MarketFinalized(MarketFinalizedEvent),
    MakerRegistered(MakerRegisteredEvent),
    PositionSettled(PositionSettledEvent),
    PositionLiquidated(PositionLiquidatedEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionOpenedEvent {
    pub signature: String,
    pub slot: Slot,
    pub market: MarketId,
    pub maker: String,
    pub taker: String,
    pub position_type: PositionType,
    pub strike: Strike,
    pub quantity: Quantity,
    pub price: Price,
    pub order_id: OrderId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCreatedEvent {
    pub signature: String,
    pub slot: Slot,
    pub market: MarketId,
    pub underlying_mint: String,
    pub quote_mint: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketFinalizedEvent {
    pub signature: String,
    pub slot: Slot,
    pub market: MarketId,
    pub settlement_price: Price,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerRegisteredEvent {
    pub signature: String,
    pub slot: Slot,
    pub owner: String,
    pub maker_pda: String,
    pub quote_signing: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionSettledEvent {
    pub signature: String,
    pub slot: Slot,
    pub position: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionLiquidatedEvent {
    pub signature: String,
    pub slot: Slot,
    pub position: String,
}
