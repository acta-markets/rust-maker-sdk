use std::time::SystemTime;

use crate::types::ids::{
    MarketId, Nonce, OrderId, PositionType, Price, Quantity, Strike, TimeoutSeconds,
};
use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};
use strum::IntoStaticStr;
use uuid::Uuid;

use super::common::{MarketDescriptor, WsChannel, default_true};

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
    GetMakerBalances(GetMakerBalancesMessage),
    GetTokenCaps(GetTokenCapsMessage),
    GetMyCaps(GetMyCapsMessage),
    GetMyTrades(GetMyTradesMessage),
    GetEarnSummary(GetEarnSummaryMessage),
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

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetMarketsMessage {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetActiveRfqsMessage {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetMakerBalancesMessage {
    pub request_id: Uuid,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetSubscriptionsMessage {
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetMyActiveRfqsMessage {
    pub request_id: Uuid,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetMyQuotesMessage {
    pub request_id: Uuid,
    #[serde(default = "default_true")]
    pub active_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CancelAllQuotesMessage {
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
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
pub struct GetTokenMarketsInfoMessage {
    pub request_id: Uuid,
    pub underlying_mint: String,
}

impl ClientMessage {
    pub fn request_id(&self) -> Option<Uuid> {
        match self {
            Self::CancelQuote(m) => Some(m.request_id),
            Self::CancelRfq(m) => Some(m.request_id),
            Self::Subscribe(m) => Some(m.request_id),
            Self::Unsubscribe(m) => Some(m.request_id),
            Self::AddMints(m) => Some(m.request_id),
            Self::RemoveMints(m) => Some(m.request_id),
            Self::AddChannels(m) => Some(m.request_id),
            Self::RemoveChannels(m) => Some(m.request_id),
            Self::CancelAllQuotes(m) => Some(m.request_id),
            Self::GetPositions(m) => Some(m.request_id),
            Self::GetMarkets(m) => Some(m.request_id),
            Self::GetMarketDescriptors(m) => Some(m.request_id),
            Self::GetExpiries(m) => Some(m.request_id),
            Self::GetTokens(m) => Some(m.request_id),
            Self::GetActiveRfqs(m) => Some(m.request_id),
            Self::GetMakerPositions(m) => Some(m.request_id),
            Self::GetMyQuotes(m) => Some(m.request_id),
            Self::GetMarketsForMaker(m) => Some(m.request_id),
            Self::GetMakerBalances(m) => Some(m.request_id),
            Self::GetMyActiveRfqs(m) => Some(m.request_id),
            Self::GetOrderStatus(m) => Some(m.request_id),
            Self::GetIndicativePrices(m) => Some(m.request_id),
            Self::GetSubscriptions(m) => Some(m.request_id),
            Self::GetTokenCaps(m) => Some(m.request_id),
            Self::GetMyCaps(m) => Some(m.request_id),
            Self::GetMyTrades(m) => Some(m.request_id),
            Self::GetEarnSummary(m) => Some(m.request_id),
            Self::GetTokenMarketsInfo(m) => Some(m.request_id),
            Self::IndicativePricesResponse(m) => Some(m.request_id),
            _ => None,
        }
    }

    pub fn expected_response_type(&self) -> Option<&'static str> {
        match self {
            Self::Quote(_) => Some("QuoteAcknowledged"),
            Self::ReplaceQuote(_) => Some("QuoteAcknowledged"),
            Self::BatchQuotes(_) => Some("BatchQuotesAck"),
            Self::CancelQuote(_) => Some("QuoteCancelled"),
            Self::CancelAllQuotes(_) => Some("CancelAllQuotesAck"),
            Self::Subscribe(_) => Some("SubscribeAck"),
            Self::Unsubscribe(_) => Some("UnsubscribeAck"),
            Self::AddMints(_) => Some("SubscriptionUpdated"),
            Self::RemoveMints(_) => Some("SubscriptionUpdated"),
            Self::AddChannels(_) => Some("SubscriptionUpdated"),
            Self::RemoveChannels(_) => Some("SubscriptionUpdated"),
            Self::GetPositions(_) => Some("Positions"),
            Self::GetMarkets(_) => Some("Markets"),
            Self::GetMarketDescriptors(_) => Some("MarketDescriptors"),
            Self::GetExpiries(_) => Some("Expiries"),
            Self::GetTokens(_) => Some("Tokens"),
            Self::GetActiveRfqs(_) => Some("ActiveRfqs"),
            Self::GetMyActiveRfqs(_) => Some("MyActiveRfqs"),
            Self::GetMakerPositions(_) => Some("MakerPositions"),
            Self::GetMyQuotes(_) => Some("MyQuotes"),
            Self::GetMarketsForMaker(_) => Some("MakerMarkets"),
            Self::GetMakerBalances(_) => Some("MakerBalances"),
            Self::GetSubscriptions(_) => Some("Subscriptions"),
            Self::GetOrderStatus(_) => Some("OrderStatus"),
            Self::GetIndicativePrices(_) => Some("IndicativePrices"),
            Self::GetTokenCaps(_) => Some("TokenCaps"),
            Self::GetMyCaps(_) => Some("MyCaps"),
            Self::GetMyTrades(_) => Some("MyTrades"),
            Self::GetEarnSummary(_) => Some("EarnSummary"),
            Self::GetTokenMarketsInfo(_) => Some("TokenMarketsInfo"),
            _ => None,
        }
    }
}
