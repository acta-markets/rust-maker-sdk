use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use crate::types::QuoteExpiry;
use crate::types::ids::{Nonce, OrderId, Price, Quantity, Strike};
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
    pub strike: Strike,
    pub min_valid_until: QuoteExpiry,
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
    #[serde_as(as = "UnixSeconds")]
    pub filled_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteExpiredMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub reason: String,
}

/// `cap_exceeded` carries the failing dimension itself (`{"cap_exceeded": <CapError>}`
/// on the wire); every other reason is a bare string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QuoteRejectReason {
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
    InternalError,
    #[serde(other)]
    Unknown,
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

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BatchQuoteResult {
    Acknowledged(QuoteAcknowledgedMessage),
    Rejected(QuoteRejectedMessage),
    Unknown(UnknownBatchQuoteResult),
}

/// A future batch-result status whose payload is not understood by this SDK.
/// The raw payload is retained so correlation can still use its `order_id`.
#[derive(Debug, Clone)]
pub struct UnknownBatchQuoteResult {
    pub status: String,
    pub data: Option<serde_json::Value>,
}

impl BatchQuoteResult {
    #[must_use]
    pub fn order_id(&self) -> Option<OrderId> {
        match self {
            Self::Acknowledged(message) => Some(message.order_id),
            Self::Rejected(message) => Some(message.order_id),
            Self::Unknown(message) => message
                .data
                .as_ref()?
                .get("order_id")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
        }
    }
}

impl Serialize for BatchQuoteResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let (status, data) = match self {
            Self::Acknowledged(message) => (
                "acknowledged",
                Some(serde_json::to_value(message).map_err(serde::ser::Error::custom)?),
            ),
            Self::Rejected(message) => (
                "rejected",
                Some(serde_json::to_value(message).map_err(serde::ser::Error::custom)?),
            ),
            Self::Unknown(message) => (message.status.as_str(), message.data.clone()),
        };
        let mut envelope = serde_json::Map::with_capacity(2);
        envelope.insert(
            "status".to_string(),
            serde_json::Value::String(status.to_string()),
        );
        if let Some(data) = data {
            envelope.insert("data".to_string(), data);
        }
        serde_json::Value::Object(envelope).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BatchQuoteResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            status: String,
            #[serde(default)]
            data: Option<serde_json::Value>,
        }

        let wire = Wire::deserialize(deserializer)?;
        match wire.status.as_str() {
            "acknowledged" => {
                let data = wire
                    .data
                    .ok_or_else(|| serde::de::Error::missing_field("data"))?;
                serde_json::from_value(data)
                    .map(Self::Acknowledged)
                    .map_err(serde::de::Error::custom)
            }
            "rejected" => {
                let data = wire
                    .data
                    .ok_or_else(|| serde::de::Error::missing_field("data"))?;
                serde_json::from_value(data)
                    .map(Self::Rejected)
                    .map_err(serde::de::Error::custom)
            }
            _ => Ok(Self::Unknown(UnknownBatchQuoteResult {
                status: wire.status,
                data: wire.data,
            })),
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
    pub strike: Strike,
    pub maker: String,
    pub price: Price,
    pub valid_until: QuoteExpiry,
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
