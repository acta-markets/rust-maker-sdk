use crate::types::unix_time::{UnixMillis, UnixSeconds};
use std::time::SystemTime;

use crate::types::ids::{
    MarketId, OrderId, PositionType, Price, Quantity, QuoteCount, RfqVersion, Strike,
};
use crate::types::{QuoteFinalStatus, RfqAvailableAgainReason, RfqCloseReason};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use uuid::Uuid;

use super::super::common::{MarketDescriptor, RfqOrderOption};

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqCreatedMessage {
    pub rfq_id: Uuid,
    pub rfq_version: RfqVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<Uuid>,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_options: Vec<RfqOrderOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedYourQuote {
    pub order_id: OrderId,
    pub status: QuoteFinalStatus,
    pub price: Price,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedWinner {
    pub maker: String,
    pub price: Price,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedMessage {
    pub rfq_id: Uuid,
    pub rfq_version: RfqVersion,
    pub reason: RfqCloseReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub your_quote: Option<RfqClosedYourQuote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<RfqClosedWinner>,
    #[serde_as(as = "UnixSeconds")]
    pub closed_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqBroadcastMessage {
    pub rfq_id: Uuid,
    pub market: MarketDescriptor,
    pub position_type: PositionType,
    pub strike: Strike,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub taker: String,
    pub order_options: Vec<RfqOrderOption>,
    /// When the server emitted this broadcast.
    ///
    /// `expires_at` is whole seconds, so on its own it cannot distinguish an
    /// RFQ that arrived promptly from one delayed behind a slow fan-out.
    /// Compare against [`ManagedInbound::received_at`] to measure delivery.
    ///
    /// [`ManagedInbound::received_at`]: crate::ws::managed::ManagedInbound
    #[serde_as(as = "UnixMillis")]
    pub sent_at_unix_ms: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqAvailableAgainMessage {
    pub rfq_id: Uuid,
    pub rfq_version: RfqVersion,
    pub reason: RfqAvailableAgainReason,
    #[serde_as(as = "UnixSeconds")]
    pub available_again_at: SystemTime,
}

/// Notification sent when an RFQ is pre-filtered due to caps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqSkippedMessage {
    pub rfq_id: Uuid,
    pub market_id: MarketId,
    pub quantity: Quantity,
    pub reason: String,
    /// Structured cap error with current/limit amounts, when the skip was a
    /// cap pre-filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_detail: Option<crate::types::errors::CapError>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRfqInfo {
    pub rfq_id: Uuid,
    pub market: MarketId,
    /// Taker pubkey: an `order_id` preimage input, so a re-read RFQ can be quoted.
    pub taker: String,
    pub position_type: PositionType,
    pub strike: Strike,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub quotes_count: QuoteCount,
    pub best_price: Option<Price>,
    pub order_options: Vec<RfqOrderOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRfqsData {
    pub request_id: Uuid,
    pub rfqs: Vec<ActiveRfqInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MyActiveRfqState {
    Active,
    PendingSignature,
    Enqueued,
    #[serde(other)]
    Unknown,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyActiveRfqInfo {
    pub rfq_id: Uuid,
    pub market: MarketId,
    pub position_type: PositionType,
    pub strike: Strike,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub state: MyActiveRfqState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_order_id: Option<OrderId>,
    pub quotes_count: QuoteCount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyActiveRfqsData {
    pub request_id: Uuid,
    pub rfqs: Vec<MyActiveRfqInfo>,
}
