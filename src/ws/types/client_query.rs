use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};
use uuid::Uuid;

use crate::types::{MarketId, OrderId, PositionType};

use super::common::default_true;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetPositionsMessage {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<TimestampSeconds<i64>>")]
    pub min_expiry_ts: Option<SystemTime>,
}

impl Default for GetPositionsMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            market: None,
            underlying_mint: None,
            status: None,
            min_expiry_ts: None,
        }
    }
}

macro_rules! default_request {
    ($type:ty) => {
        impl Default for $type {
            fn default() -> Self {
                Self {
                    request_id: Uuid::new_v4(),
                }
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMarketsMessage {
    pub request_id: Uuid,
}
default_request!(GetMarketsMessage);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetActiveRfqsMessage {
    pub request_id: Uuid,
}
default_request!(GetActiveRfqsMessage);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTokenCapsMessage {
    pub request_id: Uuid,
    #[serde(default)]
    pub include_markets: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMyCapsMessage {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubscriptionsMessage {
    pub request_id: Uuid,
}
default_request!(GetSubscriptionsMessage);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMarketDescriptorsMessage {
    pub request_id: Uuid,
    #[serde(default = "default_true")]
    pub active_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTokensMessage {
    pub request_id: Uuid,
    #[serde(default = "default_true")]
    pub active_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetExpiriesMessage {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_put: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOrderStatusMessage {
    pub request_id: Uuid,
    pub order_id: OrderId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMyActiveRfqsMessage {
    pub request_id: Uuid,
}
default_request!(GetMyActiveRfqsMessage);

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMakerPositionsMessage {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<TimestampSeconds<i64>>")]
    pub min_expiry_ts: Option<SystemTime>,
    /// Max positions to return; server clamps to `[1, 500]`, default 100.
    /// The `MakerPositions` response sets `has_more` when truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl Default for GetMakerPositionsMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            market: None,
            underlying_mint: None,
            status: None,
            min_expiry_ts: None,
            limit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMyQuotesMessage {
    pub request_id: Uuid,
    #[serde(default = "default_true")]
    pub active_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl Default for GetMyQuotesMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            active_only: true,
            limit: None,
        }
    }
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMarketsForMakerMessage {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_mints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<TimestampSeconds<i64>>")]
    pub min_expiry_ts: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<TimestampSeconds<i64>>")]
    pub max_expiry_ts: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_put: Option<bool>,
    #[serde(default)]
    pub include_stats: bool,
}

impl Default for GetMarketsForMakerMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            underlying_mints: None,
            quote_mints: None,
            min_expiry_ts: None,
            max_expiry_ts: None,
            is_put: None,
            include_stats: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelAllQuotesMessage {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
}

impl Default for CancelAllQuotesMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            market: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetIndicativePricesMessage {
    pub request_id: Uuid,
    pub market: MarketId,
    pub position_type: PositionType,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMyTradesMessage {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<TimestampSeconds<i64>>")]
    pub cursor: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetEarnSummaryMessage {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMmSummaryMessage {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTokenMarketsInfoMessage {
    pub request_id: Uuid,
    pub underlying_mint: String,
}
