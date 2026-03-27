use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use crate::ws::error::{WsClientError, WsResult};
use crate::ws::types::{
    AcceptQuoteMessage, AddChannelsData, AddMintsData, AuthChallengeData, CancelAllQuotesMessage,
    CancelQuoteData, CancelRfqData, ClientMessage, GetActiveRfqsMessage, GetExpiriesMessage,
    GetIndicativePricesMessage, GetMakerBalancesMessage, GetMakerPositionsMessage,
    GetMarketDescriptorsMessage, GetMarketsForMakerMessage, GetMarketsMessage,
    GetMyActiveRfqsMessage, GetMyQuotesMessage, GetOrderStatusMessage, GetPositionsMessage,
    GetSubscriptionsMessage, GetTokensMessage, HelloData, IndicativePricesResponseMessage,
    QuoteMessage, RemoveChannelsData, RemoveMintsData, ResumeAuthData, RfqRequestMessage,
    ServerMessage, StartAuthData, SubmitSignedSponsoredTxData, SubscribeData, UnsubscribeData,
    WsChannel,
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

impl WsClient {
    pub async fn connect(url: &str) -> WsResult<Self> {
        let (stream, _resp) = connect_async(url).await?;
        Ok(Self { stream })
    }

    pub async fn send(&mut self, msg: &ClientMessage) -> WsResult<()> {
        let json = serde_json::to_string(msg)?;
        self.stream.send(Message::Text(json.into())).await?;
        Ok(())
    }

    ws_method!(send_hello, Hello, HelloData);
    ws_method!(resume_auth, ResumeAuth, ResumeAuthData);
    ws_method!(auth_challenge, AuthChallenge, AuthChallengeData);
    ws_method!(quote, Quote, QuoteMessage);
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

    ws_method_request_id!(
        get_maker_balances,
        GetMakerBalances,
        GetMakerBalancesMessage
    );
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
                    let parsed = serde_json::from_str::<ServerMessage>(&text);
                    return Some(parsed.map_err(WsClientError::from));
                }
                Message::Binary(bin) => {
                    let parsed = serde_json::from_slice::<ServerMessage>(&bin);
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
