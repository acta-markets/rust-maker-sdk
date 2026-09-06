use crate::types::QuoteExpiry;
use crate::types::ids::{
    MarketId, Nonce, OrderId, PositionType, Price, Quantity, Strike, TimeoutSeconds,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use strum::IntoStaticStr;
use uuid::Uuid;

use super::client_query::*;
use super::common::{MarketDescriptor, WsChannel};

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr)]
#[serde(tag = "type", content = "data")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum ClientMessage {
    Hello(HelloData),
    StartAuth(StartAuthData),
    ResumeAuth(ResumeAuthData),
    Logout,
    AuthChallenge(AuthChallengeData),
    Quote(QuoteMessage),
    ReplaceQuote(ReplaceQuoteMessage),
    BatchQuotes(BatchQuotesMessage),
    CancelQuote(CancelQuoteData),
    IndicativePricesResponse(IndicativePricesResponseMessage),
    RfqRequest(RfqRequestMessage),
    AcceptQuote(AcceptQuoteMessage),
    SubmitSignedSponsoredTx(SubmitSignedSponsoredTxData),
    CancelRfq(CancelRfqData),
    GetIndicativePrices(GetIndicativePricesMessage),
    GetPositions(GetPositionsMessage),
    GetMyActiveRfqs(GetMyActiveRfqsMessage),
    GetOrderStatus(GetOrderStatusMessage),
    GetMarkets(GetMarketsMessage),
    GetMarketDescriptors(GetMarketDescriptorsMessage),
    GetExpiries(GetExpiriesMessage),
    GetTokens(GetTokensMessage),
    GetActiveRfqs(GetActiveRfqsMessage),
    GetMakerPositions(GetMakerPositionsMessage),
    GetMyQuotes(GetMyQuotesMessage),
    GetMarketsForMaker(GetMarketsForMakerMessage),
    GetTokenCaps(GetTokenCapsMessage),
    GetMyCaps(GetMyCapsMessage),
    CheckQuote(CheckQuoteMessage),
    GetMyTrades(GetMyTradesMessage),
    GetEarnSummary(GetEarnSummaryMessage),
    GetMmSummary(GetMmSummaryMessage),
    GetTokenMarketsInfo(GetTokenMarketsInfoMessage),
    GetSubscriptions(GetSubscriptionsMessage),
    CancelAllQuotes(CancelAllQuotesMessage),
    Ping,
    Subscribe(SubscribeData),
    Unsubscribe(UnsubscribeData),
    AddMints(AddMintsData),
    RemoveMints(RemoveMintsData),
    AddChannels(AddChannelsData),
    RemoveChannels(RemoveChannelsData),
    RedeemInvite(RedeemInviteData),
    ClaimReferralCode(ClaimReferralCodeData),
    GetMyReferralInfo(GetMyReferralInfoData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloData {
    pub protocol_version: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartAuthData {
    pub pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeAuthData {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthChallengeData {
    pub challenge: String,
    pub signature: String,
    pub pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelQuoteData {
    pub rfq_id: Uuid,
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitSignedSponsoredTxData {
    pub order_id: OrderId,
    pub tx_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelRfqData {
    pub rfq_id: Uuid,
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeData {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_mints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeData {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMintsData {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_mints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveMintsData {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlying_mints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_mints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddChannelsData {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveChannelsData {
    pub request_id: Uuid,
    pub channels: Vec<WsChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedeemInviteData {
    pub request_id: Uuid,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimReferralCodeData {
    pub request_id: Uuid,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMyReferralInfoData {
    pub request_id: Uuid,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteMessage {
    pub rfq_id: Uuid,
    pub strike: Strike,
    pub price: Price,
    pub valid_until: QuoteExpiry,
    pub nonce: Nonce,
    pub order_id: OrderId,
    pub signature: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceQuoteMessage {
    pub old_order_id: OrderId,
    pub rfq_id: Uuid,
    pub strike: Strike,
    pub price: Price,
    pub valid_until: QuoteExpiry,
    pub nonce: Nonce,
    pub order_id: OrderId,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchQuotesMessage {
    pub quotes: Vec<QuoteMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqRequestMessage {
    pub market: MarketId,
    pub position_type: PositionType,
    pub strike: Strike,
    pub quantity: Quantity,
    pub timeout_seconds: TimeoutSeconds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptQuoteMessage {
    pub rfq_id: Uuid,
    pub maker: String,
    pub order_id: OrderId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicativePricesRequestMessage {
    pub request_id: Uuid,
    pub market: MarketDescriptor,
    pub position_type: PositionType,
    pub strikes: Vec<Strike>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicativePricesResponseMessage {
    pub request_id: Uuid,
    pub market: MarketId,
    pub position_type: PositionType,
    pub prices: Vec<IndicativeStrikePrice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicativeStrikePrice {
    pub strike: Strike,
    pub price: Price,
}

impl ClientMessage {
    #[must_use]
    pub const fn request_id(&self) -> Option<Uuid> {
        match self {
            Self::GetPositions(m) => Some(m.request_id),
            Self::GetMyActiveRfqs(m) => Some(m.request_id),
            Self::GetOrderStatus(m) => Some(m.request_id),
            Self::GetMarkets(m) => Some(m.request_id),
            Self::GetMarketDescriptors(m) => Some(m.request_id),
            Self::GetExpiries(m) => Some(m.request_id),
            Self::GetTokens(m) => Some(m.request_id),
            Self::GetActiveRfqs(m) => Some(m.request_id),
            Self::GetMakerPositions(m) => Some(m.request_id),
            Self::GetMyQuotes(m) => Some(m.request_id),
            Self::GetMyTrades(m) => Some(m.request_id),
            Self::GetMarketsForMaker(m) => Some(m.request_id),
            Self::GetSubscriptions(m) => Some(m.request_id),
            Self::GetTokenCaps(m) => Some(m.request_id),
            Self::GetMyCaps(m) => Some(m.request_id),
            Self::CheckQuote(m) => Some(m.request_id),
            Self::GetEarnSummary(m) => Some(m.request_id),
            Self::GetMmSummary(m) => Some(m.request_id),
            Self::GetTokenMarketsInfo(m) => Some(m.request_id),
            Self::GetIndicativePrices(m) => Some(m.request_id),
            Self::CancelQuote(m) => Some(m.request_id),
            Self::CancelRfq(m) => Some(m.request_id),
            Self::CancelAllQuotes(m) => Some(m.request_id),
            Self::Subscribe(m) => Some(m.request_id),
            Self::Unsubscribe(m) => Some(m.request_id),
            Self::AddMints(m) => Some(m.request_id),
            Self::RemoveMints(m) => Some(m.request_id),
            Self::AddChannels(m) => Some(m.request_id),
            Self::RemoveChannels(m) => Some(m.request_id),
            Self::GetMyReferralInfo(m) => Some(m.request_id),
            Self::RedeemInvite(m) => Some(m.request_id),
            Self::ClaimReferralCode(m) => Some(m.request_id),
            Self::Hello(_)
            | Self::StartAuth(_)
            | Self::ResumeAuth(_)
            | Self::Logout
            | Self::AuthChallenge(_)
            | Self::Quote(_)
            | Self::ReplaceQuote(_)
            | Self::BatchQuotes(_)
            | Self::IndicativePricesResponse(_)
            | Self::RfqRequest(_)
            | Self::AcceptQuote(_)
            | Self::SubmitSignedSponsoredTx(_)
            | Self::Ping => None,
        }
    }
}

/// Transport lane. Control frames must never queue behind quote traffic.
#[cfg(feature = "ws-client")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    Control,
    Data,
}

#[cfg(feature = "ws-client")]
#[derive(Debug, Clone)]
pub(crate) enum SubscriptionChange {
    Subscribe(SubscribeData),
    Unsubscribe(UnsubscribeData),
    AddChannels(AddChannelsData),
    RemoveChannels(RemoveChannelsData),
    AddMints(AddMintsData),
    RemoveMints(RemoveMintsData),
}

#[cfg(feature = "ws-client")]
impl SubscriptionChange {
    pub(crate) fn from_message(message: &ClientMessage) -> Option<Self> {
        match message {
            ClientMessage::Subscribe(data) => Some(Self::Subscribe(data.clone())),
            ClientMessage::Unsubscribe(data) => Some(Self::Unsubscribe(data.clone())),
            ClientMessage::AddChannels(data) => Some(Self::AddChannels(data.clone())),
            ClientMessage::RemoveChannels(data) => Some(Self::RemoveChannels(data.clone())),
            ClientMessage::AddMints(data) => Some(Self::AddMints(data.clone())),
            ClientMessage::RemoveMints(data) => Some(Self::RemoveMints(data.clone())),
            _ => None,
        }
    }

    pub(crate) fn request_id(&self) -> Uuid {
        match self {
            Self::Subscribe(data) => data.request_id,
            Self::Unsubscribe(data) => data.request_id,
            Self::AddChannels(data) => data.request_id,
            Self::RemoveChannels(data) => data.request_id,
            Self::AddMints(data) => data.request_id,
            Self::RemoveMints(data) => data.request_id,
        }
    }
}

#[cfg(feature = "ws-client")]
impl ClientMessage {
    pub(crate) const fn requires_ready(&self) -> bool {
        matches!(
            self,
            Self::Quote(_)
                | Self::BatchQuotes(_)
                | Self::ReplaceQuote(_)
                | Self::CancelQuote(_)
                | Self::CancelAllQuotes(_)
                | Self::GetOrderStatus(_)
                | Self::Subscribe(_)
                | Self::Unsubscribe(_)
                | Self::AddChannels(_)
                | Self::RemoveChannels(_)
                | Self::AddMints(_)
                | Self::RemoveMints(_)
        )
    }

    pub(crate) const fn lane(&self) -> Lane {
        match self {
            Self::Hello(_)
            | Self::StartAuth(_)
            | Self::ResumeAuth(_)
            | Self::AuthChallenge(_)
            | Self::Logout
            | Self::Ping
            | Self::CancelRfq(_) => Lane::Control,
            _ => Lane::Data,
        }
    }
}
