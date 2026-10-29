use crate::Address32;
use crate::types::unix_time::{UnixMillis, UnixSeconds};
use std::time::SystemTime;

use crate::types::ids::{
    MarketId, OptionTerms, OrderId, Price, Quantity, QuoteCount, RfqVersion, TakerSide,
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
    pub terms: OptionTerms,
    pub side: TakerSide,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub request_id: Option<Uuid>,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,

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
    pub maker: Address32,
    pub price: Price,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub tx_signature: Option<String>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedMessage {
    pub rfq_id: Uuid,
    pub rfq_version: RfqVersion,
    pub reason: RfqCloseReason,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub your_quote: Option<RfqClosedYourQuote>,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub winner: Option<RfqClosedWinner>,
    #[serde_as(as = "UnixSeconds")]
    pub closed_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqBroadcastMessage {
    pub rfq_id: Uuid,
    pub market: MarketDescriptor,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub taker: Address32,
    pub order_options: Vec<RfqOrderOption>,
    /// Server emission time; compare with [`ManagedInbound::received_at`](crate::ws::managed::ManagedInbound::received_at) to measure delivery delay.
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
    /// Pre-broadcast, so `maker_insufficient_balance.required` is always `0`.
    pub cap_detail: crate::types::errors::CapError,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRfqInfo {
    pub rfq_id: Uuid,
    pub market: MarketId,
    /// Taker public key included in the `order_id` preimage.
    pub taker: Address32,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub quotes_count: QuoteCount,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
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
pub enum MyActiveRfqState {
    Active,
    PendingSignature,
    Enqueued,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyActiveRfqInfo {
    pub rfq_id: Uuid,
    pub market: MarketId,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub state: MyActiveRfqState,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub locked_order_id: Option<OrderId>,
    pub quotes_count: QuoteCount,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub best_price: Option<Price>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyActiveRfqsData {
    pub request_id: Uuid,
    pub rfqs: Vec<MyActiveRfqInfo>,
}
