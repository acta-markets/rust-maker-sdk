use std::time::SystemTime;

use crate::types::ids::{
    MarketId, Nonce, OrderId, PositionType, Price, Quantity, Strike, TimeoutSeconds,
};
use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};
use strum::IntoStaticStr;
use uuid::Uuid;

use super::client_query::*;
use super::common::{MarketDescriptor, WsChannel};

#[derive(Debug, Clone, Serialize, Deserialize, IntoStaticStr)]
#[serde(tag = "type", content = "data")]
#[strum(serialize_all = "snake_case")]
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
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub valid_until: SystemTime,
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
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub valid_until: SystemTime,
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
