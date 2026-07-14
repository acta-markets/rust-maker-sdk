use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;

use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, Utf8Bytes, protocol::WebSocketConfig},
};

use crate::ws::error::{WsClientError, WsResult, WsTransportConfigError};
use crate::ws::types::{
    AcceptQuoteMessage, AddChannelsData, AddMintsData, AuthChallengeData, BatchQuotesMessage,
    CancelAllQuotesMessage, CancelQuoteData, CancelRfqData, ClaimReferralCodeData, ClientMessage,
    GetActiveRfqsMessage, GetExpiriesMessage, GetIndicativePricesMessage, GetMakerPositionsMessage,
    GetMarketDescriptorsMessage, GetMarketsForMakerMessage, GetMarketsMessage, GetMmSummaryMessage,
    GetMyActiveRfqsMessage, GetMyQuotesMessage, GetMyReferralInfoData, GetOrderStatusMessage,
    GetPositionsMessage, GetSubscriptionsMessage, GetTokensMessage, HelloData,
    IndicativePricesResponseMessage, QuoteMessage, RedeemInviteData, RemoveChannelsData,
    RemoveMintsData, ReplaceQuoteMessage, ResumeAuthData, RfqRequestMessage, ServerMessage,
    StartAuthData, SubmitSignedSponsoredTxData, SubscribeData, UnsubscribeData, WsChannel,
    parse_server_message,
};
use uuid::Uuid;

macro_rules! ws_method {
    ($name:ident, $variant:ident, $data:ty) => {
        pub async fn $name(&mut self, data: $data) -> WsResult<()> {
            self.send(&ClientMessage::$variant(data)).await
        }
    };
}

macro_rules! ws_method_request_id {
    ($name:ident, $variant:ident, $msg:ident) => {
        pub async fn $name(&mut self) -> WsResult<()> {
            self.send(&ClientMessage::$variant($msg {
                request_id: Uuid::new_v4(),
            }))
            .await
        }
    };
}

pub struct WsClient {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

#[derive(Debug, Clone)]
pub struct PreparedClientMessage {
    text: Utf8Bytes,
}

impl PreparedClientMessage {
    pub fn new(message: &ClientMessage) -> Result<Self, serde_json::Error> {
        Ok(Self {
            text: serde_json::to_string(message)?.into(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WsTransportConfig {
    pub connect_timeout: Duration,
    pub read_buffer_size: usize,
    pub write_buffer_size: usize,
    pub max_write_buffer_size: usize,
    pub max_message_size: usize,
    pub max_frame_size: usize,
    pub tcp_nodelay: bool,
}

impl Default for WsTransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_buffer_size: 64 * 1024,
            write_buffer_size: 0,
            max_write_buffer_size: 1024 * 1024,
            max_message_size: 2 * 1024 * 1024,
            max_frame_size: 2 * 1024 * 1024,
            tcp_nodelay: true,
        }
    }
}

impl WsTransportConfig {
    /// Validate limits before a connection task is spawned.
    ///
    /// # Errors
    /// Returns a typed error for zero deadlines/limits or an invalid write-buffer range.
    pub fn validate(self) -> Result<(), WsTransportConfigError> {
        if self.connect_timeout.is_zero() {
            return Err(WsTransportConfigError::ZeroConnectTimeout);
        }
        if self.max_write_buffer_size <= self.write_buffer_size {
            return Err(WsTransportConfigError::InvalidWriteBufferRange);
        }
        if self.max_message_size == 0 || self.max_frame_size == 0 {
            return Err(WsTransportConfigError::ZeroMessageOrFrameLimit);
        }
        Ok(())
    }
}

impl WsClient {
    pub async fn connect(url: &str) -> WsResult<Self> {
        Self::connect_with_config(url, WsTransportConfig::default()).await
    }

    pub async fn connect_with_config(url: &str, config: WsTransportConfig) -> WsResult<Self> {
        config.validate()?;
        let websocket_config = WebSocketConfig::default()
            .read_buffer_size(config.read_buffer_size)
            .write_buffer_size(config.write_buffer_size)
            .max_write_buffer_size(config.max_write_buffer_size)
            .max_message_size(Some(config.max_message_size))
            .max_frame_size(Some(config.max_frame_size));
        let connect = connect_async_with_config(url, Some(websocket_config), config.tcp_nodelay);
        let (stream, _response) = tokio::time::timeout(config.connect_timeout, connect)
            .await
            .map_err(|_| WsClientError::ConnectTimeout)??;
        Ok(Self { stream })
    }

    pub async fn send(&mut self, msg: &ClientMessage) -> WsResult<()> {
        let prepared = PreparedClientMessage::new(msg)?;
        self.send_prepared(&prepared).await?;
        Ok(())
    }

    pub async fn send_prepared(&mut self, message: &PreparedClientMessage) -> WsResult<()> {
        self.stream
            .send(Message::Text(message.text.clone()))
            .await?;
        Ok(())
    }

    ws_method!(send_hello, Hello, HelloData);
    ws_method!(resume_auth, ResumeAuth, ResumeAuthData);
    ws_method!(auth_challenge, AuthChallenge, AuthChallengeData);
    ws_method!(quote, Quote, QuoteMessage);
    ws_method!(replace_quote, ReplaceQuote, ReplaceQuoteMessage);
    ws_method!(batch_quotes, BatchQuotes, BatchQuotesMessage);
    ws_method!(subscribe, Subscribe, SubscribeData);
    ws_method!(unsubscribe, Unsubscribe, UnsubscribeData);
    ws_method!(get_my_quotes, GetMyQuotes, GetMyQuotesMessage);
    ws_method!(
        get_maker_positions,
        GetMakerPositions,
        GetMakerPositionsMessage
    );
    ws_method!(
        get_markets_for_maker,
        GetMarketsForMaker,
        GetMarketsForMakerMessage
    );
    ws_method!(rfq_request, RfqRequest, RfqRequestMessage);
    ws_method!(accept_quote, AcceptQuote, AcceptQuoteMessage);
    ws_method!(
        submit_signed_sponsored_tx,
        SubmitSignedSponsoredTx,
        SubmitSignedSponsoredTxData
    );
    ws_method!(
        indicative_prices_response,
        IndicativePricesResponse,
        IndicativePricesResponseMessage
    );
    ws_method!(
        get_indicative_prices,
        GetIndicativePrices,
        GetIndicativePricesMessage
    );
    ws_method!(
        get_market_descriptors,
        GetMarketDescriptors,
        GetMarketDescriptorsMessage
    );
    ws_method!(get_tokens, GetTokens, GetTokensMessage);
    ws_method!(get_expiries, GetExpiries, GetExpiriesMessage);
    ws_method!(get_order_status, GetOrderStatus, GetOrderStatusMessage);
    ws_method!(redeem_invite, RedeemInvite, RedeemInviteData);
    ws_method!(
        claim_referral_code,
        ClaimReferralCode,
        ClaimReferralCodeData
    );
    ws_method!(
        get_my_referral_info,
        GetMyReferralInfo,
        GetMyReferralInfoData
    );

    ws_method_request_id!(get_mm_summary, GetMmSummary, GetMmSummaryMessage);
    ws_method_request_id!(get_subscriptions, GetSubscriptions, GetSubscriptionsMessage);
    ws_method_request_id!(get_markets, GetMarkets, GetMarketsMessage);

    pub async fn get_positions(&mut self) -> WsResult<()> {
        self.send(&ClientMessage::GetPositions(GetPositionsMessage {
            request_id: Uuid::new_v4(),
            ..Default::default()
        }))
        .await
    }
    ws_method_request_id!(get_my_active_rfqs, GetMyActiveRfqs, GetMyActiveRfqsMessage);
    ws_method_request_id!(get_active_rfqs, GetActiveRfqs, GetActiveRfqsMessage);

    pub async fn ping(&mut self) -> WsResult<()> {
        self.send(&ClientMessage::Ping).await
    }

    pub async fn logout(&mut self) -> WsResult<()> {
        self.send(&ClientMessage::Logout).await
    }

    pub async fn start_auth(&mut self, pubkey: String) -> WsResult<()> {
        self.send(&ClientMessage::StartAuth(StartAuthData { pubkey }))
            .await
    }

    pub async fn cancel_quote(&mut self, rfq_id: Uuid) -> WsResult<()> {
        self.send(&ClientMessage::CancelQuote(CancelQuoteData {
            rfq_id,
            request_id: Uuid::new_v4(),
        }))
        .await
    }

    pub async fn cancel_all_quotes(&mut self, market: Option<String>) -> WsResult<()> {
        self.send(&ClientMessage::CancelAllQuotes(CancelAllQuotesMessage {
            request_id: Uuid::new_v4(),
            market,
        }))
        .await
    }

    pub async fn cancel_rfq(&mut self, rfq_id: Uuid) -> WsResult<()> {
        self.send(&ClientMessage::CancelRfq(CancelRfqData {
            rfq_id,
            request_id: Uuid::new_v4(),
        }))
        .await
    }

    pub async fn add_mints(
        &mut self,
        underlying_mints: Option<Vec<String>>,
        quote_mints: Option<Vec<String>>,
    ) -> WsResult<()> {
        self.send(&ClientMessage::AddMints(AddMintsData {
            request_id: Uuid::new_v4(),
            underlying_mints,
            quote_mints,
        }))
        .await
    }

    pub async fn remove_mints(
        &mut self,
        underlying_mints: Option<Vec<String>>,
        quote_mints: Option<Vec<String>>,
    ) -> WsResult<()> {
        self.send(&ClientMessage::RemoveMints(RemoveMintsData {
            request_id: Uuid::new_v4(),
            underlying_mints,
            quote_mints,
        }))
        .await
    }

    pub async fn add_channels(&mut self, channels: Vec<WsChannel>) -> WsResult<()> {
        self.send(&ClientMessage::AddChannels(AddChannelsData {
            request_id: Uuid::new_v4(),
            channels,
        }))
        .await
    }

    pub async fn remove_channels(&mut self, channels: Vec<WsChannel>) -> WsResult<()> {
        self.send(&ClientMessage::RemoveChannels(RemoveChannelsData {
            request_id: Uuid::new_v4(),
            channels,
        }))
        .await
    }

    pub async fn send_text(&mut self, text: impl Into<String>) -> WsResult<()> {
        self.stream.send(Message::Text(text.into().into())).await?;
        Ok(())
    }

    pub async fn next(&mut self) -> Option<Result<ServerMessage, WsClientError>> {
        loop {
            let msg = match self.stream.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(err)) => return Some(Err(err.into())),
                None => return None,
            };

            match msg {
                Message::Text(text) => {
                    let parsed = parse_server_message(&text);
                    return Some(parsed.map_err(WsClientError::from));
                }
                Message::Binary(bin) => {
                    let parsed = match std::str::from_utf8(&bin) {
                        Ok(text) => parse_server_message(text),
                        Err(_) => serde_json::from_slice::<ServerMessage>(&bin),
                    };
                    return Some(parsed.map_err(WsClientError::from));
                }
                Message::Ping(payload) => {
                    if let Err(err) = self.stream.send(Message::Pong(payload)).await {
                        return Some(Err(err.into()));
                    }
                }
                Message::Pong(_) => {}
                Message::Close(_) => return None,
                Message::Frame(_) => {}
            }
        }
    }

    pub async fn close(mut self) -> WsResult<()> {
        self.stream.close(None).await?;
        Ok(())
    }
}
