use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display};
use uuid::Uuid;

use super::ids::{Balance, DurationSeconds, OrderId, Quantity};

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error(transparent)]
    Rfq(#[from] RfqError),

    #[error(transparent)]
    Quote(#[from] QuoteError),

    #[error(transparent)]
    Order(#[from] OrderError),

    #[error(transparent)]
    Market(#[from] MarketError),

    #[error("{0}")]
    RateLimit(RateLimitReason),

    #[error("{0}")]
    Cap(CapError),

    #[error(transparent)]
    System(#[from] SystemError),
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum AuthError {
    #[error("Must be authenticated to {action}")]
    Unauthenticated { action: AuthRequiredAction },

    #[error("Only {role} can {action}")]
    Unauthorized {
        role: UserRole,
        action: AuthRequiredAction,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("RFQ {rfq_id}: {kind}")]
pub struct RfqError {
    pub rfq_id: Uuid,
    #[source]
    pub kind: RfqErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("Quote in RFQ {rfq_id}{}: {kind}", .order_id.as_ref().map(|id| format!(" (order: {id})")).unwrap_or_default())]
pub struct QuoteError {
    pub rfq_id: Uuid,
    pub order_id: Option<OrderId>,
    #[source]
    pub kind: QuoteErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error(
    "Order{}{}: {kind}",
    .rfq_id
        .as_ref()
        .map(|id| format!(" in RFQ {id}"))
        .unwrap_or_default(),
    .order_id
        .as_ref()
        .map(|id| format!(" (order: {id})"))
        .unwrap_or_default()
)]
pub struct OrderError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfq_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<OrderId>,
    #[source]
    pub kind: OrderErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum MarketError {
    #[error("Market underlying oracle not available yet (listener sync; retry)")]
    OracleNotReady,

    #[error("Oracle price stale for underlying oracle (age={age_seconds}s)")]
    OraclePriceStale { age_seconds: DurationSeconds },

    #[error("Oracle price not ready for underlying oracle")]
    OraclePriceNotReady,

    #[error("Invalid market pubkey: {pubkey}")]
    InvalidMarket { pubkey: String },

    #[error("Position type must be 'covered_call' or 'cash_secured_put'")]
    InvalidPositionType,

    #[error(
        "MarketDescriptors requires underlying/quote oracle PDAs + decimals for all active markets; missing: {details}"
    )]
    MarketMetadataIncomplete { details: String },

    #[error("Tokens requires decimals for all active markets; missing: {details}")]
    TokenMetadataIncomplete { details: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
// Variants share suffix to match server-side rate limit naming convention.
#[allow(clippy::enum_variant_names)]
pub enum RateLimitReason {
    TooManyActiveRfqsPerTaker,
    TooManyActiveRfqsTotal,
    TooManyQuotesPerRfq,
    TooManySessionsPerUser,
}

impl std::fmt::Display for RateLimitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::TooManyActiveRfqsPerTaker => "Too many active RFQs for your account",
            Self::TooManyActiveRfqsTotal => "System capacity reached, please try again later",
            Self::TooManyQuotesPerRfq => "Too many quotes for this RFQ",
            Self::TooManySessionsPerUser => "Too many active sessions for your account",
        };
        f.write_str(msg)
    }
}

/// Cap violation: protocol-level exposure limits.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CapError {
    #[strum(serialize = "token_oi_cap_exceeded")]
    #[error("Token OI cap exceeded for {underlying_mint} ({current}/{limit})")]
    TokenOiCapExceeded {
        underlying_mint: String,
        current: Quantity,
        limit: Quantity,
    },

    #[strum(serialize = "market_oi_cap_exceeded")]
    #[error("Market OI cap exceeded for {market_id} ({current}/{limit})")]
    MarketOiCapExceeded {
        market_id: String,
        current: Quantity,
        limit: Quantity,
    },

    #[strum(serialize = "maker_position_cap_exceeded")]
    #[error("Maker position cap exceeded ({current}/{limit})")]
    MakerPositionCapExceeded { current: u32, limit: u32 },

    #[strum(serialize = "maker_notional_cap_exceeded")]
    #[error("Maker notional cap exceeded for {underlying_mint} ({current}/{limit})")]
    MakerNotionalCapExceeded {
        underlying_mint: String,
        current: Quantity,
        limit: Quantity,
    },

    #[strum(serialize = "maker_insufficient_balance")]
    #[error("Maker insufficient balance ({available} available, {required} required)")]
    MakerInsufficientBalance {
        available: Balance,
        required: Balance,
    },

    #[strum(serialize = "quote_notional_cap_exceeded")]
    #[error("Quote notional cap exceeded for {quote_mint} ({current}/{limit})")]
    QuoteNotionalCapExceeded {
        quote_mint: String,
        current: Balance,
        limit: Balance,
    },
}

impl CapError {
    #[must_use]
    pub fn code(&self) -> &str {
        self.as_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum SystemError {
    #[error("Unexpected kernel response")]
    InternalError,

    #[error("Kernel not available")]
    KernelNotAvailable,

    #[error("{feature} requires DB")]
    DbDisabled { feature: DbFeature },

    #[error("Server is shutting down")]
    ServerShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum UserRole {
    Maker,
    Taker,
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AuthRequiredAction {
    SubmitQuotes,
    CancelQuotes,
    QueryQuotes,
    CreateRfqs,
    AcceptQuotes,
    SubmitSignedTx,
    CancelRfqs,
    AccessRfq,
    RequestPositions,
    Subscribe,
    Unsubscribe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RfqStateError {
    NotActive,
    NotPendingSignature,
    CannotBeCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum QuoteLockedReason {
    RfqLocked,
    OrderSubmitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum DbFeature {
    MakerPositions,
    MakerMarkets,
    MarketDescriptors,
    Expiries,
    Tokens,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[serde(rename_all = "snake_case")]
pub enum RfqErrorKind {
    #[strum(serialize = "rfq_not_found")]
    #[error("not found")]
    NotFound,

    #[strum(serialize = "rfq_not_active")]
    #[error("not accepting quotes")]
    NotActive,

    #[strum(serialize = "rfq_already_locked")]
    #[error("rfq already locked")]
    AlreadyLocked,

    #[strum(serialize = "duplicate_rfq_id")]
    #[error("duplicate RFQ ID")]
    DuplicateId,

    #[strum(serialize = "rfq_invalid_state")]
    #[error("{0}")]
    InvalidState(RfqStateError),
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[serde(rename_all = "snake_case")]
pub enum QuoteErrorKind {
    #[strum(serialize = "quote_not_found")]
    #[error("not found")]
    NotFound,

    #[strum(serialize = "quote_expired")]
    #[error("expired")]
    Expired,

    #[strum(serialize = "quote_locked")]
    #[error("locked: {0}")]
    Locked(QuoteLockedReason),

    #[strum(serialize = "invalid_strike")]
    #[error("invalid strike")]
    InvalidStrike,

    #[strum(serialize = "invalid_valid_until")]
    #[error("valid_until is invalid")]
    InvalidValidUntil,

    #[strum(serialize = "quote_refresh_required")]
    #[error("quote refresh required")]
    RefreshRequired,

    #[strum(serialize = "duplicate_quote")]
    #[error("duplicate quote order_id")]
    Duplicate,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[serde(rename_all = "snake_case")]
pub enum OrderErrorKind {
    #[strum(serialize = "unknown_order")]
    #[error("unknown order")]
    UnknownOrder,

    #[strum(serialize = "order_id_mismatch")]
    #[error("ID mismatch")]
    IdMismatch,

    #[strum(serialize = "signature_timeout")]
    #[error("signature timeout")]
    SignatureTimeout,

    #[strum(serialize = "tx_build_failed")]
    #[error("sponsored TX build failed: {reason}")]
    TxBuildFailed {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },

    #[strum(serialize = "order_already_submitted")]
    #[error("order already submitted")]
    AlreadySubmitted,
}
