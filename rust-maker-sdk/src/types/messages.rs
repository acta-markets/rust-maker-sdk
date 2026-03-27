use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::{TimestampMilliSeconds, TimestampSeconds, serde_as};
use strum::IntoStaticStr;
use uuid::Uuid;

use super::errors::ServerError;
use super::ids::{
    Balance, MarketId, Nonce, OrderId, PositionType, Price, Quantity, QuoteCount, RfqVersion,
    Strike, UserId,
};

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr)]
#[serde(tag = "type", content = "data")]
#[strum(serialize_all = "snake_case")]
pub enum ServerMessage {
    RfqCreated(RfqCreatedMessage),
    RfqClosed(RfqClosedMessage),

    QuoteAcknowledged(QuoteAcknowledgedMessage),
    QuoteBestStatus(QuoteBestStatusMessage),
    QuoteOutbid(QuoteOutbidMessage),
    QuoteFilled(QuoteFilledMessage),
    QuoteSelected(QuoteSelectedMessage),
    QuoteCancelled(QuoteCancelledMessage),
    QuoteRefreshRequested(QuoteRefreshRequestedMessage),
    RfqAvailableAgain(RfqAvailableAgainMessage),

    QuoteReceived(QuoteReceivedMessage),

    ActiveRfqs(ActiveRfqsMessage),
    MyQuotes(MyQuotesMessage),

    OrderAccepted(OrderAcceptedMessage),
    SponsoredTxToSign(SponsoredTxToSignMessage),
    OrderSubmitted(OrderSubmittedMessage),
    OrderConfirmed(OrderConfirmedMessage),
    OrderFailed(OrderFailedMessage),

    Pong(PongMessage),
    Error(ServerError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqOrderOption {
    pub strike: Strike,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRfqInfo {
    pub rfq_id: Uuid,
    pub market: MarketId,
    pub position_type: PositionType,
    pub strike: Strike,
    pub quantity: Quantity,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub expires_at: SystemTime,
    pub quotes_count: QuoteCount,
    pub best_price: Option<Price>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_options: Vec<RfqOrderOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuoteStatus {
    Pending,
    Best,
    Outbid,
    Filled,
    Expired,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerQuoteInfo {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub market: MarketId,
    pub strike: Strike,
    pub price: Price,
    pub quantity: Quantity,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub valid_until: SystemTime,
    pub status: QuoteStatus,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub created_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyQuotesMessage {
    pub quotes: Vec<MakerQuoteInfo>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqCreatedMessage {
    pub rfq_id: Uuid,
    #[serde(default)]
    pub rfq_version: RfqVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<Uuid>,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub expires_at: SystemTime,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub created_at: SystemTime,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_options: Vec<RfqOrderOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RfqCloseReason {
    Expired,
    TakerCancelled,
    Filled,
    MarketExpired,
    LadderTimeout,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedMessage {
    pub rfq_id: Uuid,
    #[serde(default)]
    pub rfq_version: RfqVersion,
    pub reason: RfqCloseReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub your_quote: Option<RfqClosedYourQuote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<RfqClosedWinner>,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub closed_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum QuoteFinalStatus {
    Expired,
    Outbid,
    Cancelled,
    Filled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedYourQuote {
    pub order_id: OrderId,
    pub status: QuoteFinalStatus,
    pub price: Price,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedWinner {
    pub maker: UserId,
    pub price: Price,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteAcknowledgedMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_order_id: Option<OrderId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteBestStatusMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub is_best: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_best_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteOutbidMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub your_price: Price,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_best_price: Option<Price>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteFilledMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub taker: UserId,
    pub price: Price,
    pub quantity: Quantity,
    pub strike: Strike,
    pub position_pda: String,
    pub tx_signature: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub filled_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteReceivedMessage {
    pub rfq_id: Uuid,
    pub strike: Strike,
    pub maker: UserId,
    pub price: Price,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub valid_until: SystemTime,
    pub nonce: Nonce,
    pub order_id: OrderId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRefreshRequestedMessage {
    pub rfq_id: Uuid,
    pub strike: Strike,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub min_valid_until: SystemTime,
    pub reason: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteSelectedMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub taker: UserId,
    pub price: Price,
    pub quantity: Quantity,
    pub strike: Strike,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub signature_deadline: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum QuoteCancelReason {
    Requested,
    RiskCheck,
    RfqAccepted,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteCancelledMessage {
    pub rfq_id: Uuid,
    #[serde(default)]
    pub order_ids: Vec<OrderId>,
    pub reason: QuoteCancelReason,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub cancelled_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RfqAvailableAgainReason {
    SignatureTimeout,
    TxFailed,
    TxBuildFailed,
}

impl std::fmt::Display for RfqAvailableAgainReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.into())
    }
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqAvailableAgainMessage {
    pub rfq_id: Uuid,
    #[serde(default)]
    pub rfq_version: RfqVersion,
    pub reason: RfqAvailableAgainReason,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub available_again_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRfqsMessage {
    pub rfqs: Vec<ActiveRfqInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderAcceptedMessage {
    pub order_id: OrderId,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SponsoredTxToSignMessage {
    pub order_id: OrderId,
    pub tx_base64: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub signature_deadline: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSubmittedMessage {
    pub order_id: OrderId,
    pub tx_signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderConfirmedMessage {
    pub order_id: OrderId,
    pub position_pda: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFailedMessage {
    pub order_id: OrderId,
    pub reason: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PongMessage {
    #[serde_as(as = "TimestampMilliSeconds<i64>")]
    pub server_time_unix_ms: SystemTime,
}

// =============================================================================
// Caps
// =============================================================================

/// Open interest cap status for a single underlying token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCapInfo {
    pub underlying_mint: String,
    pub symbol: String,
    pub current_oi: Quantity,
    pub max_oi: Quantity,
    pub utilization: f64,
}

/// Open interest cap status for a single market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCapInfo {
    pub market_id: MarketId,
    pub current_oi: Quantity,
    pub max_oi: Quantity,
    pub utilization: f64,
}

/// Maker's personal position cap status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerPositionCapInfo {
    pub current: u32,
    pub limit: u32,
}

/// Maker's notional exposure cap per underlying.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerNotionalCapInfo {
    pub underlying_mint: String,
    pub symbol: String,
    pub current: Quantity,
    pub limit: Quantity,
}

/// Maker's available balance for quoting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerBalanceCapInfo {
    pub mint: String,
    pub symbol: String,
    pub deposited: Balance,
    pub committed: Balance,
    pub available: Balance,
}

/// Quote-level notional cap status for a single quote mint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteCapInfo {
    pub quote_mint: String,
    pub symbol: String,
    /// Current total premium committed for this quote mint.
    pub current_notional: Balance,
    /// Maximum allowed notional (budget). `0` means unlimited.
    pub max_notional: Balance,
    pub utilization: f64,
}
