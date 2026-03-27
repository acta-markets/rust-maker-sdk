mod server_auth;
mod server_query;
mod server_quote;
mod server_rfq;

pub use server_auth::*;
pub use server_query::*;
pub use server_quote::*;
pub use server_rfq::*;

use std::time::SystemTime;

use crate::types::CapError;
use crate::types::ids::DurationSeconds;
use serde::{Deserialize, Serialize};
use serde_with::{TimestampMilliSeconds, serde_as};
use strum::IntoStaticStr;
use uuid::Uuid;

use super::common::WsChannel;
use super::market::MarketInfo;

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    Welcome(WelcomeData),
    VersionMismatch(VersionMismatchData),
    AuthRequest(AuthRequestData),
    AuthSuccess(AuthSuccessData),
    AuthError(AuthErrorData),
    LogoutSuccess(LogoutSuccessData),
    RfqCreated(RfqCreatedMessage),
    RfqClosed(RfqClosedMessage),
    RfqBroadcast(RfqBroadcastMessage),
    QuoteSelected(QuoteSelectedMessage),
    QuoteCancelled(QuoteCancelledMessage),
    QuoteReceived(QuoteReceivedMessage),
    QuotesUpdate(QuotesUpdateMessage),
    IndicativePrices(IndicativePricesMessage),
    QuoteRefreshRequested(QuoteRefreshRequestedMessage),
    IndicativePricesRequest(super::client::IndicativePricesRequestMessage),
    QuoteAcknowledged(QuoteAcknowledgedMessage),
    QuoteBestStatus(QuoteBestStatusMessage),
    QuoteOutbid(QuoteOutbidMessage),
    QuoteFilled(QuoteFilledMessage),
    RfqAvailableAgain(RfqAvailableAgainMessage),
    QuoteExpired(QuoteExpiredMessage),
    QuoteRejected(QuoteRejectedMessage),
    ActiveRfqs(ActiveRfqsData),
    MakerPositions(MakerPositionsMessage),
    MyQuotes(MyQuotesMessage),
    MakerMarkets(MakerMarketsMessage),
    MakerBalances(MakerBalancesMessage),
    TokenCaps(TokenCapsData),
    MyCaps(MyCapsData),
    MyTrades(MyTradesMessage),
    EarnSummary(EarnSummaryData),
    TokenMarketsInfo(TokenMarketsInfoData),
    RfqSkipped(RfqSkippedMessage),
    CancelAllQuotesAck(CancelAllQuotesAckMessage),
    BatchQuotesAck(BatchQuotesAckMessage),
    Subscriptions(SubscriptionsMessage),
    MyActiveRfqs(MyActiveRfqsData),
    OrderStatus(OrderStatusMessage),
    OrderAccepted(OrderAcceptedData),
    SponsoredTxToSign(SponsoredTxToSignData),
    OrderSubmitted(OrderSubmittedData),
    OrderConfirmed(OrderConfirmedData),
    OrderFailed(OrderFailedData),
    MarketCreated(MarketInfo),
    MarketFinalized(MarketFinalizedData),
    ChainEvent(ChainEventMessage),
    Snapshot(SnapshotMessage),
    Positions(PositionsData),
    Markets(MarketsData),
    MarketDescriptors(MarketDescriptorsData),
    Expiries(ExpiriesData),
    Tokens(TokensData),
    TradeExecuted(TradeExecutedMessage),
    PositionUpdated(PositionUpdatedMessage),
    StatsUpdate(StatsUpdateMessage),
    Pong(PongData),
    Error(ServerError),
    /// Request-level error: `request_id` echoes back the client's request.
    RequestError(RequestErrorEnvelope),
    SubscribeAck(SubscribeAckData),
    UnsubscribeAck(UnsubscribeAckData),
    SubscriptionUpdated(SubscriptionUpdatedData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestErrorEnvelope {
    pub request_id: Uuid,
    pub error: ServerError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeAckData {
    pub request_id: Uuid,
    pub subscribed: Vec<WsChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeAckData {
    pub request_id: Uuid,
    pub unsubscribed: Vec<WsChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr, thiserror::Error)]
#[serde(tag = "type", content = "data")]
#[strum(serialize_all = "snake_case")]
pub enum ServerError {
    #[error("Must be authenticated to {action}")]
    Unauthenticated {
        action: super::common::AuthRequiredAction,
    },

    #[error("Only {role} can {action}")]
    Unauthorized {
        role: super::common::UserRole,
        action: super::common::AuthRequiredAction,
    },

    #[error("RFQ not found")]
    RfqNotFound,

    #[error("RFQ is not accepting quotes")]
    RfqNotActive,

    #[error("RFQ is already locked by another order")]
    RfqAlreadyLocked,

    #[error("{state}")]
    InvalidState { state: super::common::RfqStateError },

    #[error("Cannot cancel quote: {reason}")]
    QuoteLocked {
        reason: super::common::QuoteLockedReason,
    },

    #[error("Quote not found")]
    QuoteNotFound,

    #[error("Quote has expired")]
    QuoteExpired,

    #[error("Quote valid_until too short; must be at least {min_seconds}s from now")]
    QuoteExpiryTooShort { min_seconds: u32 },

    #[error("Strike is not an allowed option for this RFQ")]
    InvalidStrike,

    #[error("Quote valid_until is invalid")]
    InvalidValidUntil,

    #[error("Order ID does not match")]
    OrderIdMismatch,

    #[error("Unknown order ID")]
    UnknownOrder,

    #[error("Signature timeout; please select another quote")]
    SignatureTimeout,

    #[error("Market underlying oracle not available yet (listener sync; retry)")]
    OracleNotReady,

    #[error("Oracle price stale for underlying oracle (age={age_seconds}s)")]
    OraclePriceStale { age_seconds: DurationSeconds },

    #[error("Oracle price not ready for underlying oracle")]
    OraclePriceNotReady,

    #[error("Position type must be 'covered_call' or 'cash_secured_put'")]
    InvalidPositionType,

    #[error("Invalid market pubkey: {pubkey}")]
    InvalidMarket { pubkey: String },

    #[error(
        "MarketDescriptors requires underlying/quote oracle PDAs + decimals for all active markets; missing: {details}"
    )]
    MarketMetadataIncomplete { details: String },

    #[error("Tokens requires decimals for all active markets; missing: {details}")]
    TokenMetadataIncomplete { details: String },

    #[error("{0}")]
    RateLimit(super::common::RateLimitReason),

    #[error("{0}")]
    Cap(CapError),

    #[error("Unexpected kernel response")]
    InternalError,

    #[error("Kernel not available")]
    KernelNotAvailable,

    #[error("{feature} requires DB")]
    DbDisabled { feature: super::common::DbFeature },

    #[error("Server is shutting down")]
    ServerShuttingDown,

    #[error("{message}")]
    Generic { code: String, message: String },
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WelcomeData {
    pub protocol_version: String,
    pub server_version: String,
    pub min_supported_version: String,
    pub enabled_features: Vec<String>,
    #[serde_as(as = "Option<TimestampMilliSeconds<i64>>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_time_unix_ms: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionMismatchData {
    pub requested_version: String,
    pub server_version: String,
    pub min_supported_version: String,
    pub message: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PongData {
    #[serde_as(as = "TimestampMilliSeconds<i64>")]
    pub server_time_unix_ms: SystemTime,
}

impl ServerMessage {
    pub fn request_id(&self) -> Option<Uuid> {
        match self {
            Self::CancelAllQuotesAck(m) => Some(m.request_id),
            Self::SubscribeAck(m) => Some(m.request_id),
            Self::UnsubscribeAck(m) => Some(m.request_id),
            Self::SubscriptionUpdated(m) => Some(m.request_id),
            Self::Subscriptions(m) => Some(m.request_id),
            Self::Positions(m) => Some(m.request_id),
            Self::Markets(m) => Some(m.request_id),
            Self::MarketDescriptors(m) => Some(m.request_id),
            Self::Expiries(m) => Some(m.request_id),
            Self::Tokens(m) => Some(m.request_id),
            Self::ActiveRfqs(m) => Some(m.request_id),
            Self::MyActiveRfqs(m) => Some(m.request_id),
            Self::MakerPositions(m) => Some(m.request_id),
            Self::MyQuotes(m) => Some(m.request_id),
            Self::MyTrades(m) => Some(m.request_id),
            Self::MakerMarkets(m) => Some(m.request_id),
            Self::MakerBalances(m) => Some(m.request_id),
            Self::OrderStatus(m) => Some(m.request_id),
            Self::IndicativePrices(m) => Some(m.request_id),
            Self::TokenCaps(m) => Some(m.request_id),
            Self::MyCaps(m) => Some(m.request_id),
            Self::EarnSummary(m) => Some(m.request_id),
            Self::TokenMarketsInfo(m) => Some(m.request_id),
            Self::RequestError(env) => Some(env.request_id),
            _ => None,
        }
    }
}
