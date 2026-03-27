use std::time::SystemTime;

use crate::types::ids::{Nonce, OrderId, Price, Quantity, Strike};
use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};
use strum::IntoStaticStr;
use uuid::Uuid;

use super::super::common::QuoteCancelReason;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRefreshRequestedMessage {
    pub rfq_id: Uuid,
    pub strike: Strike,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub min_valid_until: SystemTime,
    pub reason: String,
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
    pub taker: String,
    pub price: Price,
    pub quantity: Quantity,
    pub strike: Strike,
    pub position_pda: String,
    pub tx_signature: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub filled_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteExpiredMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
pub enum QuoteRejectReason {
    InvalidStrike,
    MarketExpired,
    QuoteExpiryTooShort,
    InvalidSignature,
    MakerNotRegistered,
    OrderIdMismatch,
    CapExceeded,
    RfqNotFound,
    RfqNotActive,
    DuplicateOrderId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRejectedMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub reason: QuoteRejectReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteSelectedMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub taker: String,
    pub price: Price,
    pub quantity: Quantity,
    pub strike: Strike,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub signature_deadline: SystemTime,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelAllQuotesAckMessage {
    pub request_id: Uuid,
    pub cancelled_count: u32,
    pub cancelled_order_ids: Vec<OrderId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "data")]
pub enum BatchQuoteResult {
    #[serde(rename = "acknowledged")]
    Acknowledged(QuoteAcknowledgedMessage),
    #[serde(rename = "rejected")]
    Rejected(QuoteRejectedMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchQuotesAckMessage {
    pub results: Vec<BatchQuoteResult>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteReceivedMessage {
    pub rfq_id: Uuid,
    pub strike: Strike,
    pub maker: String,
    pub price: Price,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub valid_until: SystemTime,
    pub nonce: Nonce,
    pub order_id: OrderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotesUpdateMessage {
    pub rfq_id: Uuid,
    pub quotes: Vec<QuoteReceivedMessage>,
}
