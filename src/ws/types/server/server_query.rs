use crate::Address32;
use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use crate::types::QuoteExpiry;
use crate::types::ids::{
    Decimals, MarketId, OptionTerms, OrderId, OrderVersion, PositionType, Price, Quantity,
    RfqVersion, Slot, Strike,
};
use crate::types::{
    MakerBalanceCapInfo, MakerNotionalCapInfo, MakerPositionCapInfo, MarketCapInfo, QuoteCapInfo,
    QuoteLiabilityCapInfo, TakerSide, TokenCapInfo,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use uuid::Uuid;

use super::super::common::{
    GlobalStats, MarketDescriptorInfo, PositionInfo, PositionSizeRule, PositionUpdateType,
    TokenInfo, TradeInfo, WsChannel,
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
    #[serde_as(as = "Vec<UnixSeconds>")]
    pub expiries_ts: Vec<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokensData {
    pub request_id: Uuid,
    pub tokens: Vec<TokenInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceChangedData {
    pub tokens_etag: String,
    pub markets_etag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerPositionsMessage {
    pub request_id: Uuid,
    pub positions: Vec<MakerPositionInfo>,
    /// Positions omitted from `positions` because their market projection is incomplete.
    pub unrenderable: Vec<UnrenderablePositionInfo>,
    /// More rows exist past this page's cursor.
    pub has_more: bool,
}

/// A stored position whose market projection is incomplete.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnrenderablePositionInfo {
    pub pda: Address32,
    pub market: MarketId,
    /// Keyset-cursor key, so a page may end on an unrenderable row.
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,
    /// Field names the projection could not supply.
    pub missing: Vec<String>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerPositionInfo {
    pub revision: crate::types::ids::PositionRevision,
    pub pda: Address32,
    pub market: MarketId,
    pub underlying_mint: Address32,
    pub underlying_symbol: String,
    pub underlying_decimals: u8,
    pub quote_mint: Address32,
    pub quote_symbol: String,
    pub quote_decimals: u8,
    pub terms: OptionTerms,
    pub side: TakerSide,
    #[serde(flatten)]
    pub state: crate::ws::types::common::PositionState,
    pub quantity: Quantity,
    pub price: Price,
    /// Net premium in quote atoms, after the protocol fee.
    pub total_premium: crate::types::ids::Balance,
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyQuotesMessage {
    pub request_id: Uuid,
    pub quotes: Vec<MakerQuoteInfo>,
    /// More historical rows exist past this page's cursor.
    pub has_more: bool,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerQuoteInfo {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub market: MarketId,
    pub underlying_mint: Address32,
    pub underlying_symbol: String,
    pub underlying_decimals: u8,
    pub quote_mint: Address32,
    pub quote_symbol: String,
    pub quote_decimals: u8,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub price: Price,
    pub quantity: Quantity,
    pub valid_until: QuoteExpiry,
    pub state: MakerQuoteState,
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,
}

/// The live book state or a persisted historical record of a maker quote.
/// `Historical` intentionally has no RFQ version: that projection is not the
/// authoritative live RFQ state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MakerQuoteState {
    Active {
        rank: QuoteRank,
        rfq_version: RfqVersion,
    },
    Retained {
        rfq_version: RfqVersion,
    },
    AwaitingSignature {
        rfq_version: RfqVersion,
    },
    Executing {
        rfq_version: RfqVersion,
    },
    Historical {
        status: HistoricalQuoteStatus,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteRank {
    Best,
    Outbid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalQuoteStatus {
    Submitted,
    Filled,
    Cancelled,
    Replaced,
    Expired,
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerMarketsMessage {
    pub request_id: Uuid,
    pub markets: Vec<MakerMarketInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerMarketInfo {
    pub products: crate::types::ProductSet,
    pub market_pda: Address32,
    pub underlying_mint: Address32,
    pub quote_mint: Address32,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub is_finalized: bool,
    pub underlying_symbol: String,
    pub quote_symbol: String,
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
    pub market_pda: Address32,
    pub underlying_mint: Address32,
    pub underlying_symbol: String,
    pub underlying_decimals: u8,
    pub quote_mint: Address32,
    pub quote_symbol: String,
    pub quote_decimals: u8,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub taker: Address32,
    pub quantity: Quantity,
    pub price: Price,
    pub tx_signature: String,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub position_pda: Option<Address32>,
    #[serde_as(as = "UnixSeconds")]
    pub confirmed_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarnSummaryData {
    pub request_id: Uuid,
    pub assets: Vec<EarnAssetSummary>,
    #[serde_as(as = "UnixSeconds")]
    pub computed_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarnAssetSummary {
    pub underlying_mint: Address32,
    pub underlying_symbol: String,
    pub quote_mint: Address32,
    pub quote_symbol: String,
    pub position_type: PositionType,
    pub apr: AprRange,
    pub cap_filled_pct: f64,
    pub cap_total: Quantity,
    pub cap_used: Quantity,
    pub strikes_count: u32,
    pub nearest_market_pda: Address32,
    pub markets_count: u32,
    #[serde_as(as = "UnixSeconds")]
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
    pub products: crate::types::ProductSet,
    pub market_pda: Address32,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub indicatives: Vec<TokenMarketIndicatives>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMarketIndicatives {
    pub position_type: PositionType,
    #[serde_as(as = "UnixSeconds")]
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
    #[serde_as(as = "UnixSeconds")]
    pub updated_at: SystemTime,
    pub is_stale: bool,
    pub strikes: Vec<IndicativeStrikeBest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicativeStrikeBest {
    pub strike: Strike,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub best_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCapsData {
    pub request_id: Uuid,
    pub tokens: Vec<TokenCapInfo>,
    pub markets: Vec<MarketCapInfo>,
    pub quotes: Vec<QuoteCapInfo>,
    pub quote_liabilities: Vec<QuoteLiabilityCapInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyCapsData {
    pub request_id: Uuid,
    pub positions: MakerPositionCapInfo,
    pub notional: Vec<MakerNotionalCapInfo>,
    pub balances: Vec<MakerBalanceCapInfo>,
    pub quote_liabilities: Vec<QuoteLiabilityCapInfo>,
}

/// Answer to `CheckQuote`. A rejection carries exactly what a `QuoteRejected`
/// would; `caps_unavailable` means retry, not a cap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteCheckData {
    pub request_id: Uuid,
    pub verdict: QuoteVerdict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuoteVerdict {
    Admissible {},
    Rejected { cap_detail: crate::types::CapError },
}

/// APR is unavailable while no strike has a fresh indicative premium.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AprRange {
    Unavailable {},
    Available { min: f64, max: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerCapsSnapshot {
    pub positions: MakerPositionCapInfo,
    pub notional: Vec<MakerNotionalCapInfo>,
    pub balances: Vec<MakerBalanceCapInfo>,
    pub quote_liabilities: Vec<QuoteLiabilityCapInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmSummaryData {
    pub request_id: Uuid,
    pub maker_pda: Address32,
    pub caps: MyCapsData,
    pub positions: Vec<MakerPositionInfo>,
    pub active_quotes: Vec<MakerQuoteInfo>,
    pub markets: Vec<MakerMarketInfo>,
    pub tokens: Vec<TokenInfo>,
    #[serde_as(as = "UnixSeconds")]
    pub computed_at: SystemTime,
    /// Positions omitted from `positions` because their market projection is incomplete.
    pub unrenderable_positions: Vec<UnrenderablePositionInfo>,
    /// True when the embedded `positions` were capped; page via `GetMakerPositions`.
    pub positions_has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionsMessage {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
    pub underlying_mints: Vec<Address32>,
    pub quote_mints: Vec<Address32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionUpdatedData {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
    pub underlying_mints: Vec<Address32>,
    pub quote_mints: Vec<Address32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderStatusMessage {
    pub request_id: Uuid,
    pub order_id: OrderId,
    pub state: OrderExecutionState,
}

/// Evidence available for an order lookup. `Unknown` means the server cannot
/// establish an owned pending execution or a confirmed position; it is not a
/// proof that the order never executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrderExecutionState {
    Pending,
    Confirmed { position_pda: Address32 },
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMessage {
    pub markets: Vec<MarketInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeExecutedMessage {
    pub trade: TradeInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionUpdatedMessage {
    pub position: PositionInfo,
    pub update_type: PositionUpdateType,
}

/// Owner-only capacity update, emitted after the new caps take effect.
/// `Lifecycle` follows settlement or liquidation; `Policy` follows an admin limit change.
/// Delivery is best-effort: refresh authoritative state after reconnect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerCapsUpdatedMessage {
    pub caps_snapshot: MakerCapsSnapshot,
    pub reason: CapsUpdateReason,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, strum::IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CapsUpdateReason {
    Lifecycle,
    Policy,
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
    pub last_valid_block_height: u64,
    #[serde_as(as = "UnixSeconds")]
    pub signature_deadline: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderAcceptedData {
    pub order_id: OrderId,
    pub order_version: OrderVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSubmittedData {
    pub order_id: OrderId,
    pub tx_signature: String,
    pub order_version: OrderVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderConfirmedData {
    pub order_id: OrderId,
    pub position_pda: Address32,
    pub order_version: OrderVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFailedData {
    pub order_id: OrderId,
    pub reason: String,
    pub order_version: OrderVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketFinalizedData {
    pub market_pda: Address32,
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

/// Known on-chain events are keyed by transaction signature and instruction index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainEventIdentity<'a> {
    pub signature: &'a str,
    pub instruction_index: i16,
}

impl ChainEventMessage {
    #[must_use]
    pub fn identity(&self) -> ChainEventIdentity<'_> {
        let (signature, instruction_index) = match self {
            Self::PositionOpened(event) => (&event.signature, event.instruction_index),
            Self::MarketCreated(event) => (&event.signature, event.instruction_index),
            Self::MarketFinalized(event) => (&event.signature, event.instruction_index),
            Self::MakerRegistered(event) => (&event.signature, event.instruction_index),
            Self::PositionSettled(event) => (&event.signature, event.instruction_index),
            Self::PositionLiquidated(event) => (&event.signature, event.instruction_index),
        };
        ChainEventIdentity {
            signature,
            instruction_index,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionOpenedEvent {
    pub signature: String,
    /// Canonical instruction coordinate within `signature`.
    pub instruction_index: i16,
    pub slot: Slot,
    pub market: MarketId,
    pub maker: Address32,
    pub taker: Address32,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    pub price: Price,
    pub order_id: OrderId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCreatedEvent {
    pub signature: String,
    pub instruction_index: i16,
    pub slot: Slot,
    pub market: MarketId,
    pub underlying_mint: Address32,
    pub quote_mint: Address32,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketFinalizedEvent {
    pub signature: String,
    pub instruction_index: i16,
    pub slot: Slot,
    pub market: MarketId,
    pub settlement_price: Price,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerRegisteredEvent {
    pub signature: String,
    pub instruction_index: i16,
    pub slot: Slot,
    pub owner: Address32,
    pub maker_pda: Address32,
    pub quote_signing: Address32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionSettledEvent {
    pub signature: String,
    pub instruction_index: i16,
    pub slot: Slot,
    pub position: Address32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionLiquidatedEvent {
    pub signature: String,
    pub instruction_index: i16,
    pub slot: Slot,
    pub position: Address32,
}
