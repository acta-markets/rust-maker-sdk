use crate::Address32;
use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use crate::types::ids::{Nonce, OptionTerms, OrderId, Price, Quantity};
use crate::types::{QuoteExpiry, QuoteExpiryReason, TakerSide};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use strum::IntoStaticStr;
use uuid::Uuid;

use crate::types::QuoteCancelReason;
use crate::types::errors::CapError;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRefreshRequestedMessage {
    pub rfq_id: Uuid,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub min_valid_until: QuoteExpiry,
    pub reason: QuoteExpiryReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteAcknowledgedMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub replaced_order_id: Option<OrderId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteBestStatusMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub is_best: bool,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub current_best_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteOutbidMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub your_price: Price,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub current_best_price: Option<Price>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteFilledMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub market_pda: Address32,
    /// `None` when the server no longer knows the market; resolve it from `market_pda`.
    #[serde_as(as = "Option<UnixSeconds>", no_default)]
    pub expiry_ts: Option<SystemTime>,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub taker: Address32,
    pub price: Price,
    pub quantity: Quantity,
    pub position_pda: Address32,
    pub tx_signature: String,
    #[serde_as(as = "UnixSeconds")]
    pub filled_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteExpiredMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub reason: QuoteExpiryReason,
}

/// `cap_exceeded` carries the failing dimension itself (`{"cap_exceeded": <CapError>}`
/// on the wire); every other reason is a bare string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
pub enum QuoteRejectReason {
    ProductNotAllowed,
    TakerBuyUnsupported,
    SpreadInvalidStrikes,
    SpreadTickViolation,
    SpreadPremiumInvalid,
    QuoteIdentityMismatch,
    Unauthorized,
    InvalidStrike,
    MarketExpired,
    QuoteExpiryTooShort,
    InvalidSignature,
    MakerNotRegistered,
    OrderIdMismatch,
    CapExceeded(CapError),
    RfqNotFound,
    RfqNotActive,
    DuplicateOrderId,
    RateLimited,
    TradingPaused,
    InternalError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRejectedMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub reason: QuoteRejectReason,
    pub message: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteSelectedMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub taker: Address32,
    pub price: Price,
    pub quantity: Quantity,
    pub terms: OptionTerms,
    pub side: TakerSide,
    #[serde_as(as = "UnixSeconds")]
    pub signature_deadline: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteCancelledMessage {
    pub rfq_id: Uuid,
    pub order_ids: Vec<OrderId>,
    pub reason: QuoteCancelReason,
    #[serde_as(as = "UnixSeconds")]
    pub cancelled_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelQuoteAckMessage {
    pub request_id: Uuid,
    pub rfq_id: Uuid,
    pub cancelled_order_ids: Vec<OrderId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelAllQuotesAckMessage {
    pub request_id: Uuid,
    pub cancelled_count: u32,
    pub cancelled_order_ids: Vec<OrderId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum BatchQuoteResult {
    Acknowledged(QuoteAcknowledgedMessage),
    Rejected(QuoteRejectedMessage),
}

impl BatchQuoteResult {
    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        match self {
            Self::Acknowledged(message) => message.order_id,
            Self::Rejected(message) => message.order_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchQuotesAckMessage {
    pub results: Vec<BatchQuoteResult>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteReceivedMessage {
    pub rfq_id: Uuid,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub maker: Address32,
    pub price: Price,
    pub valid_until: QuoteExpiry,
    pub nonce: Nonce,
    pub order_id: OrderId,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub net_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotesUpdateMessage {
    pub rfq_id: Uuid,
    pub quotes: Vec<QuoteReceivedMessage>,
}
