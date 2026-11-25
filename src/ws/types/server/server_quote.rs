use crate::Address32;
use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use crate::types::ids::{Nonce, OptionTerms, OrderId, Price, Quantity, RfqVersion};
use crate::types::{
    QuoteExpiry, QuoteExpiryReason, QuoteLockedReason, RfqAvailableAgainReason, RfqCloseReason,
    TakerSide,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use strum::IntoStaticStr;
use uuid::Uuid;

use crate::types::QuoteCancelReason;
use crate::types::errors::CapError;

/// `cap_exceeded` carries the failing dimension itself (`{"cap_exceeded": <CapError>}`
/// on the wire); every other reason is a bare string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
pub enum QuoteRejectReason {
    ProductNotAllowed,
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
    /// The RFQ is locked on a selected quote; quote again if it becomes available.
    RfqLocked,
    /// The quote this one replaces is locked in a trade.
    QuoteLocked(QuoteLockedReason),
    /// The quote named in `replaces` is no longer in the book.
    ReplacedOrderNotFound,
    InvalidValidUntil,
    /// The order id is already known: selected, ended, in another RFQ or with other terms.
    DuplicateOrderId,
    RateLimited,
    /// Transient server or market-data condition; retry later.
    Overloaded,
    TradingPaused,
    InternalError,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteReceivedMessage {
    pub rfq_id: Uuid,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub maker_owner: Address32,
    pub price: Price,
    pub valid_until: QuoteExpiry,
    pub nonce: Nonce,
    pub order_id: OrderId,
    pub net_price: Price,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotesUpdateMessage {
    pub rfq_id: Uuid,
    pub quotes: Vec<QuoteReceivedMessage>,
}

/// Every change to one of your quotes; each order ends with exactly one terminal state.
/// Book states carry the RFQ's `rfq_version`: drop one only if it is lower than what you hold
/// for that RFQ (a `GetMyQuotes` row included); one change can send several at the same version.
/// `rejected` refuses one submission: it ends the order only if you have not seen that order
/// `live` or `selected`; otherwise the order keeps its state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteUpdateMessage {
    pub rfq_id: Uuid,
    pub order_id: OrderId,
    pub state: QuoteState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuoteState {
    /// Accepted or re-ranked.
    Live {
        rfq_version: RfqVersion,
        is_best: bool,
        best_price: crate::ws::types::common::QuotedPrice,
    },
    /// The order ended; quote again with a fresh order to stay in the RFQ.
    RefreshRequested {
        rfq_version: RfqVersion,
        terms: OptionTerms,
        side: TakerSide,
        min_valid_until: QuoteExpiry,
        reason: QuoteExpiryReason,
    },
    Selected(QuoteSelection),
    Filled(QuoteFill),
    Rejected {
        reason: QuoteRejectReason,
        message: String,
    },
    Expired {
        rfq_version: RfqVersion,
        reason: QuoteExpiryReason,
    },
    /// The RFQ was selected on this quote but the trade did not happen; the quote is gone.
    Released {
        rfq_version: RfqVersion,
        reason: RfqAvailableAgainReason,
    },
    /// The RFQ closed without filling this quote.
    Lost {
        rfq_version: RfqVersion,
        reason: RfqCloseReason,
    },
    /// Withdrawn: `requested` by your own cancel, otherwise by the server.
    Cancelled {
        rfq_version: RfqVersion,
        reason: QuoteCancelReason,
    },
}

impl QuoteState {
    /// No further update can follow for this order.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        !matches!(self, Self::Live { .. } | Self::Selected(_))
    }
}

/// The taker picked this quote and has until `signature_deadline_at` to sign.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteSelection {
    pub rfq_version: RfqVersion,
    pub taker_owner: Address32,
    pub price: Price,
    pub quantity: Quantity,
    pub terms: OptionTerms,
    pub side: TakerSide,
    #[serde_as(as = "UnixSeconds")]
    pub signature_deadline_at: SystemTime,
}

/// The position this quote opened on chain.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteFill {
    pub rfq_version: RfqVersion,
    pub market_pda: Address32,
    pub terms: OptionTerms,
    pub side: TakerSide,
    pub taker_owner: Address32,
    pub price: Price,
    pub quantity: Quantity,
    pub position_pda: Address32,
    pub transaction_signature: String,
    #[serde_as(as = "UnixSeconds")]
    pub filled_at: SystemTime,
}
