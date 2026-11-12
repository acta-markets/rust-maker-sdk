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
    pub request_id: Uuid,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    #[serde_as(as = "UnixSeconds")]
    pub created_at: SystemTime,

    pub order_options: Vec<RfqOrderOption>,
}

/// The receiving maker's own quote on the closed RFQ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum YourQuote {
    None,
    Quoted {
        order_id: OrderId,
        status: QuoteFinalStatus,
        price: Price,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RfqWinner {
    None,
    Winner {
        maker_owner: Address32,
        price: Price,
        transaction: WinnerTransaction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WinnerTransaction {
    Pending,
    Sent { signature: String },
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqClosedMessage {
    pub rfq_id: Uuid,
    pub rfq_version: RfqVersion,
    pub reason: RfqCloseReason,
    pub your_quote: YourQuote,
    pub winner: RfqWinner,
    #[serde_as(as = "UnixSeconds")]
    pub closed_at: SystemTime,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqBroadcastMessage {
    pub rfq_id: Uuid,
    pub market_descriptor: MarketDescriptor,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub taker_owner: Address32,
    pub order_options: Vec<RfqOrderOption>,
    /// Server emission time; compare with [`ManagedInbound::received_at`](crate::ws::managed::ManagedInbound::received_at) to measure delivery delay.
    #[serde_as(as = "UnixMillis")]
    pub sent_at_ms: SystemTime,
}

/// Another maker's quote was accepted; the RFQ takes no quotes until it closes or becomes available again.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqLockedMessage {
    pub rfq_id: Uuid,
    pub rfq_version: RfqVersion,
    #[serde_as(as = "UnixSeconds")]
    pub locked_at: SystemTime,
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
    pub market_pda: MarketId,
    pub quantity: Quantity,
    /// Pre-broadcast, so `maker_insufficient_balance.required` is always `0`.
    pub cap_detail: crate::types::errors::CapError,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRfqInfo {
    pub rfq_id: Uuid,
    pub market_pda: MarketId,
    /// Taker public key included in the `order_id` preimage.
    pub taker_owner: Address32,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub quotes_count: QuoteCount,
    pub best_price: crate::ws::types::common::QuotedPrice,
    pub order_options: Vec<RfqOrderOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRfqsData {
    pub request_id: Uuid,
    pub rfqs: Vec<ActiveRfqInfo>,
    #[serde(deserialize_with = "crate::ws::types::common::deserialize_required_nullable")]
    pub next_cursor: Option<String>,
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
    pub market_pda: MarketId,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub quantity: Quantity,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    pub state: MyActiveRfqState,
    pub lock: RfqLock,
    pub quotes_count: QuoteCount,
    pub best_price: crate::ws::types::common::QuotedPrice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RfqLock {
    Unlocked,
    Locked { order_id: OrderId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyActiveRfqsData {
    pub request_id: Uuid,
    pub rfqs: Vec<MyActiveRfqInfo>,
    #[serde(deserialize_with = "crate::ws::types::common::deserialize_required_nullable")]
    pub next_cursor: Option<String>,
}
