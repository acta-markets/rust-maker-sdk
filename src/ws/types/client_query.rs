use super::common::{PageCursor, PositionStatus, QueryFilter, TimeBound};
use crate::Address32;

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use uuid::Uuid;

use crate::types::{MarketId, OptionTerms, OrderId, PositionType, Price, Quantity, TakerSide};

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetPositionsMessage {
    pub request_id: Uuid,
    pub market: QueryFilter<Address32>,
    pub underlying_mint: QueryFilter<Address32>,
    pub status: Vec<PositionStatus>,
    pub min_expiry_ts: TimeBound,
}

impl Default for GetPositionsMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            market: QueryFilter::All {},
            underlying_mint: QueryFilter::All {},
            status: Vec::new(),
            min_expiry_ts: TimeBound::Unbounded {},
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
    pub include_markets: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMyCapsMessage {
    pub request_id: Uuid,
}

/// Maker-only dry run of cap admission for a would-be quote: the same checks
/// `Quote` runs, reserving nothing. A passing check is advice, not a hold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckQuoteMessage {
    pub request_id: Uuid,
    pub market_id: MarketId,
    pub quantity: Quantity,
    /// Gross price per underlying unit in quote units, scaled by 1e9, as on `Quote`.
    pub price: Price,
    pub terms: OptionTerms,
    pub side: TakerSide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubscriptionsMessage {
    pub request_id: Uuid,
}
default_request!(GetSubscriptionsMessage);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMarketDescriptorsMessage {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTokensMessage {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetExpiriesMessage {
    pub request_id: Uuid,
    pub underlying_mint: QueryFilter<Address32>,
    pub quote_mint: QueryFilter<Address32>,
    pub is_put: QueryFilter<bool>,
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
    pub market: QueryFilter<Address32>,
    pub underlying_mint: QueryFilter<Address32>,
    pub status: Vec<PositionStatus>,
    pub min_expiry_ts: TimeBound,
    /// Page size in `1..=500`; anything else is rejected with `invalid_limit`.
    pub limit: u32,
    pub cursor: PageCursor<Address32>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMyQuotesMessage {
    pub request_id: Uuid,
    pub scope: MakerQuoteScope,
}

impl Default for GetMyQuotesMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            scope: MakerQuoteScope::Live {},
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MakerQuoteScope {
    Live {},
    History {
        limit: u32,
        cursor: PageCursor<OrderId>,
    },
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMarketsForMakerMessage {
    pub request_id: Uuid,
    pub underlying_mints: Vec<Address32>,
    pub quote_mints: Vec<Address32>,
    pub min_expiry_ts: TimeBound,
    pub max_expiry_ts: TimeBound,
    pub is_put: QueryFilter<bool>,
}

impl Default for GetMarketsForMakerMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            underlying_mints: Vec::new(),
            quote_mints: Vec::new(),
            min_expiry_ts: TimeBound::Unbounded {},
            max_expiry_ts: TimeBound::Unbounded {},
            is_put: QueryFilter::All {},
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelAllQuotesMessage {
    pub request_id: Uuid,
    pub market: QueryFilter<Address32>,
}

impl Default for CancelAllQuotesMessage {
    fn default() -> Self {
        Self {
            request_id: Uuid::new_v4(),
            market: QueryFilter::All {},
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
    pub limit: u32,
    pub cursor: PageCursor<Uuid>,
    pub market: QueryFilter<Address32>,
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
    pub underlying_mint: Address32,
}
