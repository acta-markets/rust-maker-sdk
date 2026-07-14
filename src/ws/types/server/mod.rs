mod server_auth;
mod server_query;
mod server_quote;
mod server_rfq;

pub use server_auth::*;
pub use server_query::*;
pub use server_quote::*;
pub use server_rfq::*;

use std::time::SystemTime;

use crate::types::ids::DurationSeconds;
use crate::types::{
    AuthRequiredAction, CapError, DbFeature, QuoteLockedReason, RateLimitReason, ReferralCode,
    RfqStateError, TakerStatus, UserRole,
};
use serde::{Deserialize, Serialize};
use serde_with::{TimestampMilliSeconds, serde_as};
use strum::IntoStaticStr;
use uuid::Uuid;

use super::common::WsChannel;
use super::market::MarketInfo;

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr, strum::VariantNames)]
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
    TokenCaps(TokenCapsData),
    MyCaps(MyCapsData),
    MyTrades(MyTradesMessage),
    EarnSummary(EarnSummaryData),
    MmSummary(MmSummaryData),
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
    RequireInvite,
    InviteRedeemed(InviteRedeemedData),
    ReferralCodeClaimed(ReferralCodeClaimedData),
    MyReferralInfo(MyReferralInfoData),
    /// Forward-compat: any message type this SDK version doesn't know yet.
    Unknown(UnknownServerMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnknownServerMessage {
    pub message_type: String,
    pub raw_json: Box<str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnknownServerError {
    pub error_type: String,
    pub raw_json: Box<str>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteRedeemedData {
    pub request_id: Uuid,
    pub referral_code: ReferralCode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferralCodeClaimedData {
    pub request_id: Uuid,
    pub referral_code: ReferralCode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyReferralInfoData {
    pub request_id: Uuid,
    pub referral_code: ReferralCode,
    pub status: TakerStatus,
    pub total_invited: i64,
    pub invited_this_period: i64,
    pub max_invites_per_period: u32,
    pub next_slot_frees_in_seconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteErrorReason {
    InvalidCode,
    CodeExhausted,
    CodeExpired,
    CodeDisabled,
    CodeOwnerInactive,
    CodeOwnerBlacklisted,
    AlreadyRegistered,
    InternalError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimErrorReason {
    NotRegistered,
    InvalidFormat,
    CodeTaken,
    Reserved,
    InternalError,
}

#[derive(
    Debug, Clone, Serialize, Deserialize, IntoStaticStr, strum::VariantNames, thiserror::Error,
)]
#[serde(tag = "type", content = "data")]
pub enum ServerError {
    #[error("Must be authenticated to {action}")]
    Unauthenticated { action: AuthRequiredAction },

    #[error("Only {role} can {action}")]
    Unauthorized {
        role: UserRole,
        action: AuthRequiredAction,
    },

    #[error("RFQ not found")]
    RfqNotFound,

    #[error("RFQ is not accepting quotes")]
    RfqNotActive,

    #[error("RFQ is already locked by another order")]
    RfqAlreadyLocked,

    #[error("{state}")]
    InvalidState { state: RfqStateError },

    #[error("Cannot cancel quote: {reason}")]
    QuoteLocked { reason: QuoteLockedReason },

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
    RateLimit(RateLimitReason),

    #[error("{0}")]
    Cap(CapError),

    #[error("Unexpected kernel response")]
    InternalError,

    #[error("Kernel not available")]
    KernelNotAvailable,

    #[error("{feature} requires DB")]
    DbDisabled { feature: DbFeature },

    #[error("Server is shutting down")]
    ServerShuttingDown,

    #[error("Invite required to trade")]
    InviteRequired,

    #[error("Invite error: {0:?}")]
    Invite(InviteErrorReason),

    #[error("Claim error: {0:?}")]
    Claim(ClaimErrorReason),

    #[error("Message is not allowed on {endpoint:?}; allowed endpoints: {allowed_endpoints:?}")]
    WrongEndpoint {
        endpoint: super::common::WsEndpointKind,
        allowed_endpoints: Vec<super::common::WsEndpointKind>,
    },

    #[error("{message}")]
    Generic { code: String, message: String },

    /// Forward-compat: any error code this SDK version doesn't know yet.
    #[error("unrecognized server error")]
    Unknown(UnknownServerError),
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
    #[must_use]
    pub const fn request_id(&self) -> Option<Uuid> {
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
            Self::OrderStatus(m) => Some(m.request_id),
            Self::IndicativePrices(m) => Some(m.request_id),
            Self::TokenCaps(m) => Some(m.request_id),
            Self::MyCaps(m) => Some(m.request_id),
            Self::EarnSummary(m) => Some(m.request_id),
            Self::MmSummary(m) => Some(m.request_id),
            Self::TokenMarketsInfo(m) => Some(m.request_id),
            Self::RequestError(env) => Some(env.request_id),
            Self::InviteRedeemed(m) => Some(m.request_id),
            Self::ReferralCodeClaimed(m) => Some(m.request_id),
            Self::MyReferralInfo(m) => Some(m.request_id),
            Self::Welcome(_)
            | Self::VersionMismatch(_)
            | Self::AuthRequest(_)
            | Self::AuthSuccess(_)
            | Self::AuthError(_)
            | Self::LogoutSuccess(_)
            | Self::RfqCreated(_)
            | Self::RfqClosed(_)
            | Self::RfqBroadcast(_)
            | Self::QuoteSelected(_)
            | Self::QuoteCancelled(_)
            | Self::QuoteReceived(_)
            | Self::QuotesUpdate(_)
            | Self::QuoteRefreshRequested(_)
            | Self::IndicativePricesRequest(_)
            | Self::QuoteAcknowledged(_)
            | Self::QuoteBestStatus(_)
            | Self::QuoteOutbid(_)
            | Self::QuoteFilled(_)
            | Self::RfqAvailableAgain(_)
            | Self::QuoteExpired(_)
            | Self::QuoteRejected(_)
            | Self::RfqSkipped(_)
            | Self::BatchQuotesAck(_)
            | Self::OrderAccepted(_)
            | Self::SponsoredTxToSign(_)
            | Self::OrderSubmitted(_)
            | Self::OrderConfirmed(_)
            | Self::OrderFailed(_)
            | Self::MarketCreated(_)
            | Self::MarketFinalized(_)
            | Self::ChainEvent(_)
            | Self::Snapshot(_)
            | Self::TradeExecuted(_)
            | Self::PositionUpdated(_)
            | Self::StatsUpdate(_)
            | Self::Pong(_)
            | Self::Error(_)
            | Self::RequireInvite
            | Self::Unknown(_) => None,
        }
    }
}

/// Parse a server frame, tolerating unknown message types and error codes
/// (forward-compat): unknown `type` → [`ServerMessage::Unknown`], unknown
/// error code → [`ServerError::Unknown`]. A known type with a malformed
/// payload is still an error.
pub fn parse_server_message(text: &str) -> Result<ServerMessage, serde_json::Error> {
    match serde_json::from_str::<ServerMessage>(text) {
        Ok(msg) => Ok(msg),
        Err(err) => match unknown_fallback(text) {
            Some(msg) => Ok(msg),
            None => Err(err),
        },
    }
}

fn unknown_fallback(text: &str) -> Option<ServerMessage> {
    use strum::VariantNames;

    #[derive(Deserialize)]
    struct Tag {
        r#type: String,
    }
    #[derive(Deserialize)]
    struct Envelope {
        data: Tag,
    }
    #[derive(Deserialize)]
    struct RequestErrorWire {
        data: RequestErrorDataWire,
    }
    #[derive(Deserialize)]
    struct RequestErrorDataWire {
        request_id: Uuid,
        error: Tag,
    }

    let tag = serde_json::from_str::<Tag>(text).ok()?;
    if !ServerMessage::VARIANTS.contains(&tag.r#type.as_str()) {
        tracing::debug!(message_type = %tag.r#type, "ignoring unknown server message type");
        return Some(ServerMessage::Unknown(UnknownServerMessage {
            message_type: tag.r#type,
            raw_json: text.into(),
        }));
    }
    match tag.r#type.as_str() {
        "Error" => {
            let env = serde_json::from_str::<Envelope>(text).ok()?;
            if ServerError::VARIANTS.contains(&env.data.r#type.as_str()) {
                return None;
            }
            tracing::debug!(error_type = %env.data.r#type, "unrecognized server error code");
            Some(ServerMessage::Error(ServerError::Unknown(
                UnknownServerError {
                    error_type: env.data.r#type,
                    raw_json: text.into(),
                },
            )))
        }
        "RequestError" => {
            let env = serde_json::from_str::<RequestErrorWire>(text).ok()?;
            if ServerError::VARIANTS.contains(&env.data.error.r#type.as_str()) {
                return None;
            }
            tracing::debug!(error_type = %env.data.error.r#type, "unrecognized server error code");
            Some(ServerMessage::RequestError(RequestErrorEnvelope {
                request_id: env.data.request_id,
                error: ServerError::Unknown(UnknownServerError {
                    error_type: env.data.error.r#type,
                    raw_json: text.into(),
                }),
            }))
        }
        _ => None,
    }
}
