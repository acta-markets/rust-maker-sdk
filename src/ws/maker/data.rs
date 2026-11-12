use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use crate::ws::managed::{
    MakerWsEndpoint, ManagedMessageReceiver, ManagedWsConfig, ManagedWsError, ManagedWsEvent,
    ManagedWsHandle, ManagedWsState, ReferenceChangeReceiver, SendAwaitError, WaitUntilReadyError,
    spawn_managed_ws,
};
use crate::ws::types::*;
use tokio::sync::broadcast;

use super::MakerClientError;

mod private {
    pub trait Sealed {}
}

/// Request with a statically known data-plane response.
pub trait MakerDataRequest: private::Sealed + Sized {
    type Response;

    const RESPONSE_TYPE: &'static str;

    fn into_message(self) -> ClientMessage;
    fn fetch(
        self,
        client: &MakerDataClient,
        timeout: Duration,
    ) -> impl std::future::Future<Output = Result<MakerResponse<Self::Response>, MakerDataRequestError>>
    {
        client.request_page(self, timeout)
    }
    /// # Errors
    /// Returns the original message when its variant is not this request's
    /// response.
    fn into_response(message: ServerMessage) -> Result<Self::Response, Box<ServerMessage>>;
}

/// A list request using the common opaque continuation contract.
pub trait PagedRequest: MakerDataRequest + Clone {
    fn first_page(&mut self);
    fn next_page(&mut self, page: &Self::Response) -> bool;
    fn append(all: &mut Self::Response, page: Self::Response);
    fn has_more(page: &Self::Response) -> bool;
    fn cursor(page: &Self::Response) -> Option<&str>;
    fn request_cursor(&mut self, cursor: Cursor);
}

macro_rules! paged_request {
    ($request:ty, $response:ty, $($field:ident).+ $(, $($rest:ident).+)* $(,)?) => {
        impl PagedRequest for $request {
            fn first_page(&mut self) {
                self.request_id = uuid::Uuid::new_v4();
                self.limit = crate::paging::MAX_PAGE_SIZE;
                self.cursor = Cursor::Start {};
            }
            fn next_page(&mut self, page: &$response) -> bool {
                let Some(id) = &page.next_cursor else { return false; };
                let cursor = Cursor::After { id: id.clone() };
                if id.is_empty() || cursor == self.cursor { return false; }
                self.cursor = cursor;
                self.request_id = uuid::Uuid::new_v4();
                true
            }
            fn append(all: &mut $response, page: $response) {
                all.$($field).+.extend(page.$($field).+);
                $(all.$($rest).+.extend(page.$($rest).+);)*
                all.next_cursor = page.next_cursor;
            }
            fn has_more(page: &$response) -> bool { page.next_cursor.is_some() }
            fn cursor(page: &$response) -> Option<&str> { page.next_cursor.as_deref() }
            fn request_cursor(&mut self, cursor: Cursor) { self.cursor = cursor; self.request_id = uuid::Uuid::new_v4(); }
        }
    };
}

paged_request!(
    GetMakerPositionsMessage,
    MakerPositionsMessage,
    positions,
    unrenderable
);
paged_request!(GetMyTradesMessage, MyTradesMessage, trades);
paged_request!(GetMyQuotesMessage, MyQuotesMessage, quotes);
paged_request!(GetMarketsMessage, MarketsData, markets);
paged_request!(GetMarketDescriptorsMessage, MarketDescriptorsData, markets);
paged_request!(GetExpiriesMessage, ExpiriesData, expiries_ts);
paged_request!(GetTokensMessage, TokensData, tokens);
paged_request!(GetActiveRfqsMessage, ActiveRfqsData, rfqs);
paged_request!(GetMyActiveRfqsMessage, MyActiveRfqsData, rfqs);
paged_request!(GetPositionsMessage, PositionsData, positions);
paged_request!(GetPositionSummaryMessage, PositionSummaryData, summaries);
paged_request!(GetIndicativePricesMessage, IndicativePricesMessage, strikes);
paged_request!(GetMarketsForMakerMessage, MakerMarketsMessage, markets);
paged_request!(
    GetTokenCapsMessage,
    TokenCapsData,
    tokens,
    markets,
    quotes,
    quote_liabilities
);
paged_request!(
    GetMyCapsMessage,
    MyCapsData,
    notional,
    balances,
    quote_liabilities
);
paged_request!(GetEarnSummaryMessage, EarnSummaryData, assets);
paged_request!(
    GetSubscriptionsMessage,
    SubscriptionsMessage,
    channels,
    underlying_mints,
    quote_mints
);
paged_request!(
    GetMmSummaryMessage,
    MmSummaryData,
    positions,
    unrenderable_positions,
    active_quotes,
    markets,
    tokens,
    caps.notional,
    caps.balances,
    caps.quote_liabilities
);

impl PagedRequest for GetTokenMarketsInfoMessage {
    fn first_page(&mut self) {
        self.request_id = uuid::Uuid::new_v4();
        self.limit = crate::paging::MAX_PAGE_SIZE;
        self.cursor = Cursor::Start {};
    }
    fn next_page(&mut self, page: &TokenMarketsInfoData) -> bool {
        let Some(id) = &page.next_cursor else {
            return false;
        };
        let cursor = Cursor::After { id: id.clone() };
        if id.is_empty() || cursor == self.cursor {
            return false;
        }
        self.cursor = cursor;
        self.request_id = uuid::Uuid::new_v4();
        true
    }
    fn has_more(page: &TokenMarketsInfoData) -> bool {
        page.next_cursor.is_some()
    }
    fn cursor(page: &TokenMarketsInfoData) -> Option<&str> {
        page.next_cursor.as_deref()
    }
    fn request_cursor(&mut self, cursor: Cursor) {
        self.cursor = cursor;
        self.request_id = uuid::Uuid::new_v4();
    }
    fn append(all: &mut TokenMarketsInfoData, page: TokenMarketsInfoData) {
        for market in page.markets {
            if let Some(existing) = all
                .markets
                .iter_mut()
                .find(|m| m.market_pda == market.market_pda)
            {
                for indicative in market.indicatives {
                    if let Some(current) = existing.indicatives.iter_mut().find(|i| {
                        i.position_type == indicative.position_type && i.side == indicative.side
                    }) {
                        current.strikes.extend(indicative.strikes);
                    } else {
                        existing.indicatives.push(indicative);
                    }
                }
            } else {
                all.markets.push(market);
            }
        }
        all.next_cursor = page.next_cursor;
    }
}

pub struct MakerResponse<T> {
    payload: T,
}

impl<T> MakerResponse<T> {
    #[must_use]
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    #[must_use]
    pub fn into_payload(self) -> T {
        self.payload
    }
}

impl<T> Deref for MakerResponse<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MakerDataRequestError {
    #[error("page claims more rows but provides no advancing cursor")]
    InvalidPage,
    #[error(transparent)]
    Continuation(#[from] crate::paging::InvalidPage),
    #[error(transparent)]
    Session(#[from] ManagedWsError),
    #[error(transparent)]
    Transport(#[from] SendAwaitError),
    #[error("server rejected request {request_id}")]
    Rejected {
        request_id: uuid::Uuid,
        response: Arc<ServerMessage>,
    },
    #[error("expected {expected}, received a different correlated response")]
    UnexpectedResponse {
        expected: &'static str,
        response: Arc<ServerMessage>,
    },
}

impl MakerDataRequestError {
    #[must_use]
    pub fn server_error(&self) -> Option<&ServerError> {
        match self {
            Self::Rejected { response, .. } => match &**response {
                ServerMessage::RequestError(error) => Some(&error.error),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Maker data-plane client.
#[derive(Clone)]
pub struct MakerDataClient {
    inner: ManagedWsHandle,
}

impl MakerDataClient {
    pub fn spawn(config: ManagedWsConfig) -> Result<Self, MakerClientError> {
        let config = Self::prepare_config(config)?;
        Ok(Self {
            inner: spawn_managed_ws(config)?,
        })
    }

    fn prepare_config(config: ManagedWsConfig) -> Result<ManagedWsConfig, MakerClientError> {
        if config.endpoint != MakerWsEndpoint::Data {
            return Err(MakerClientError::WrongEndpoint {
                client: "MakerDataClient",
                required: MakerWsEndpoint::Data,
                actual: config.endpoint,
            });
        }
        Ok(config)
    }

    /// Fetch the complete response. A list request follows every page, so use
    /// `request_page` to stop early.
    pub async fn request_all<R: MakerDataRequest>(
        &self,
        request: R,
        timeout: Duration,
    ) -> Result<MakerResponse<R::Response>, MakerDataRequestError> {
        request.fetch(self, timeout).await
    }

    /// Fetch exactly one page. Use `request_all` for a complete collection.
    pub async fn request_page<R: MakerDataRequest>(
        &self,
        request: R,
        timeout: Duration,
    ) -> Result<MakerResponse<R::Response>, MakerDataRequestError> {
        let message = self
            .inner
            .send_await_ready(request.into_message(), timeout)
            .await?;
        if let ServerMessage::RequestError(error) = &*message {
            return Err(MakerDataRequestError::Rejected {
                request_id: error.request_id,
                response: message,
            });
        }
        let message = match Arc::try_unwrap(message) {
            Ok(owned) => owned,
            Err(shared) => (*shared).clone(),
        };
        match R::into_response(message) {
            Ok(payload) => Ok(MakerResponse { payload }),
            Err(other) => Err(MakerDataRequestError::UnexpectedResponse {
                expected: R::RESPONSE_TYPE,
                response: Arc::from(other),
            }),
        }
    }

    /// Every page of `request` at the server's largest page size; its own
    /// `limit` and cursor are ignored. Pages after the first are appended.
    pub async fn all_pages<R: PagedRequest>(
        &self,
        mut request: R,
        timeout: Duration,
    ) -> Result<R::Response, MakerDataRequestError> {
        request.first_page();
        crate::paging::fetch_all(
            |cursor| {
                let mut next = request.clone();
                next.request_cursor(cursor);
                async move {
                    self.request_page(next, timeout)
                        .await
                        .map(MakerResponse::into_payload)
                }
            },
            R::cursor,
            R::append,
        )
        .await
    }

    /// Receive owner-direct `PositionUpdated`, `TradeExecuted` and
    /// `MakerCapsUpdated` pushes alongside query responses. No subscription is
    /// required. Owner pushes begin after `MmSummary` recovery; quote lifecycle
    /// messages stay on the quote session. On reconnect or a stream gap, rebuild
    /// authoritative state (for example with `maker_open_positions`).
    pub fn subscribe_messages(&self) -> ManagedMessageReceiver {
        self.inner.subscribe_messages()
    }

    pub fn subscribe_reference_changes(&self) -> ReferenceChangeReceiver {
        self.inner.subscribe_reference_changes()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ManagedWsEvent> {
        self.inner.subscribe_events()
    }

    #[must_use]
    pub fn state(&self) -> ManagedWsState {
        self.inner.state()
    }

    pub async fn wait_until_ready(&self) -> Result<u64, WaitUntilReadyError> {
        self.inner.wait_until_ready().await
    }

    #[must_use]
    pub const fn raw(&self) -> &ManagedWsHandle {
        &self.inner
    }

    #[must_use]
    pub fn into_raw(self) -> ManagedWsHandle {
        self.inner
    }

    pub async fn close(&self) -> Result<(), ManagedWsError> {
        self.inner.close().await
    }
}

macro_rules! paged_fetch {
    (paged) => {
        async fn fetch(
            self,
            client: &MakerDataClient,
            timeout: Duration,
        ) -> Result<MakerResponse<Self::Response>, MakerDataRequestError> {
            client
                .all_pages(self, timeout)
                .await
                .map(|payload| MakerResponse { payload })
        }
    };
}

macro_rules! data_request {
    ($request:ty, $client_variant:ident, $response:ty, $server_variant:ident $(, $paged:ident)?) => {
        impl private::Sealed for $request {}

        impl MakerDataRequest for $request {
            type Response = $response;

            $(paged_fetch!($paged);)?

            const RESPONSE_TYPE: &'static str = stringify!($server_variant);

            fn into_message(self) -> ClientMessage {
                ClientMessage::$client_variant(self)
            }

            fn into_response(message: ServerMessage) -> Result<Self::Response, Box<ServerMessage>> {
                match message {
                    ServerMessage::$server_variant(response) => Ok(response),
                    other => Err(Box::new(other)),
                }
            }
        }
    };
}

data_request!(
    GetActiveRfqsMessage,
    GetActiveRfqs,
    ActiveRfqsData,
    ActiveRfqs,
    paged
);
data_request!(GetMarketsMessage, GetMarkets, MarketsData, Markets, paged);
data_request!(
    GetMarketDescriptorsMessage,
    GetMarketDescriptors,
    MarketDescriptorsData,
    MarketDescriptors,
    paged
);
data_request!(
    GetExpiriesMessage,
    GetExpiries,
    ExpiriesData,
    Expiries,
    paged
);
data_request!(GetTokensMessage, GetTokens, TokensData, Tokens, paged);
data_request!(
    GetIndicativePricesMessage,
    GetIndicativePrices,
    IndicativePricesMessage,
    IndicativePrices,
    paged
);
data_request!(
    GetMakerPositionsMessage,
    GetMakerPositions,
    MakerPositionsMessage,
    MakerPositions,
    paged
);
data_request!(
    GetMyQuotesMessage,
    GetMyQuotes,
    MyQuotesMessage,
    MyQuotes,
    paged
);
data_request!(
    GetOrderStatusMessage,
    GetOrderStatus,
    OrderStatusMessage,
    OrderStatus
);
data_request!(
    GetMarketsForMakerMessage,
    GetMarketsForMaker,
    MakerMarketsMessage,
    MakerMarkets,
    paged
);
data_request!(
    GetTokenCapsMessage,
    GetTokenCaps,
    TokenCapsData,
    TokenCaps,
    paged
);
data_request!(GetMyCapsMessage, GetMyCaps, MyCapsData, MyCaps, paged);
data_request!(CheckQuoteMessage, CheckQuote, QuoteCheckData, QuoteCheck);
data_request!(
    GetMyTradesMessage,
    GetMyTrades,
    MyTradesMessage,
    MyTrades,
    paged
);
data_request!(
    GetEarnSummaryMessage,
    GetEarnSummary,
    EarnSummaryData,
    EarnSummary,
    paged
);
data_request!(
    GetMmSummaryMessage,
    GetMmSummary,
    MmSummaryData,
    MmSummary,
    paged
);
data_request!(
    GetTokenMarketsInfoMessage,
    GetTokenMarketsInfo,
    TokenMarketsInfoData,
    TokenMarketsInfo,
    paged
);

data_request!(
    GetPositionsMessage,
    GetPositions,
    PositionsData,
    Positions,
    paged
);
data_request!(
    GetPositionSummaryMessage,
    GetPositionSummary,
    PositionSummaryData,
    PositionSummary,
    paged
);
data_request!(
    GetMyActiveRfqsMessage,
    GetMyActiveRfqs,
    MyActiveRfqsData,
    MyActiveRfqs,
    paged
);
data_request!(
    GetSubscriptionsMessage,
    GetSubscriptions,
    SubscriptionsMessage,
    Subscriptions,
    paged
);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ws::managed::ManagedWsConfig;
    use tokio::sync::mpsc::error::TryRecvError;

    struct LeavesReadyWhileEncoding {
        handle: ManagedWsHandle,
        request_id: uuid::Uuid,
    }

    impl private::Sealed for LeavesReadyWhileEncoding {}

    impl MakerDataRequest for LeavesReadyWhileEncoding {
        type Response = MmSummaryData;

        const RESPONSE_TYPE: &'static str = "MmSummary";

        fn into_message(self) -> ClientMessage {
            self.handle.inject_state(ManagedWsState::Authenticated {
                connection_epoch: 7,
            });
            ClientMessage::GetMmSummary(GetMmSummaryMessage {
                limit: crate::paging::MAX_PAGE_SIZE,
                cursor: crate::ws::types::Cursor::Start {},

                request_id: self.request_id,
            })
        }

        fn into_response(message: ServerMessage) -> Result<Self::Response, Box<ServerMessage>> {
            match message {
                ServerMessage::MmSummary(response) => Ok(response),
                other => Err(Box::new(other)),
            }
        }
    }

    fn config() -> ManagedWsConfig {
        ManagedWsConfig::new(
            "ws://localhost",
            HelloData {
                protocol_version: crate::WS_PROTOCOL_VERSION.to_string(),
                on_disconnect: crate::ws::types::OnDisconnect::KeepQuotes,
                client_name: "test".to_string(),
                client_version: "test".to_string(),
            },
            Arc::new(crate::orders::BytesSigner::from_secret([1u8; 32])),
            crate::Address32::new([76; 32]),
        )
    }

    #[test]
    fn data_client_rejects_quote_endpoint_before_spawning() {
        assert!(matches!(
            MakerDataClient::spawn(config()),
            Err(MakerClientError::WrongEndpoint {
                required: MakerWsEndpoint::Data,
                actual: MakerWsEndpoint::Quote,
                ..
            })
        ));
    }

    #[test]
    fn continuations_use_the_server_key_even_without_renderable_rows() {
        let mut request = GetMyQuotesMessage::default();
        request.first_page();
        let mut all = MyQuotesMessage {
            request_id: request.request_id,
            quotes: Vec::new(),
            next_cursor: Some("opaque".into()),
        };
        assert!(request.next_page(&all));
        assert_eq!(
            request.cursor,
            Cursor::After {
                id: "opaque".into()
            }
        );
        assert!(!request.next_page(&all));
        GetMyQuotesMessage::append(
            &mut all,
            MyQuotesMessage {
                request_id: request.request_id,
                quotes: Vec::new(),
                next_cursor: None,
            },
        );
        assert!(!GetMyQuotesMessage::has_more(&all));
    }

    #[test]
    fn token_market_pages_merge_per_product_and_side() {
        let page = |side: &str, strike: u64, next: Option<&str>| -> TokenMarketsInfoData {
            serde_json::from_value(serde_json::json!({
                "next_cursor": next, "request_id": "00000000-0000-0000-0000-000000000000",
                "underlying_symbol": "SOL", "underlying_decimals": 9, "quote_symbol": "USDC", "quote_decimals": 6,
                "size_rule": { "min_size": 1, "max_size": 10, "step": 1 }, "reference_price": 1, "session": { "type": "continuous" },
                "markets": [{ "products": ["covered_call"], "market_pda": "11111111111111111111111111111111",
                    "expiry_ts": 10, "is_put": false,
                    "indicatives": [{ "position_type": "covered_call", "side": side, "freshness": { "type": "no_data" },
                        "strikes": [{ "strike": strike, "price": { "type": "quoted", "price": strike + 100 } }] }] }]
            }))
            .unwrap()
        };
        let mut all = page("sell", 1, Some("a"));
        for next in [
            page("sell", 2, Some("b")),
            page("buy", 1, Some("c")),
            page("buy", 2, None),
        ] {
            GetTokenMarketsInfoMessage::append(&mut all, next);
        }
        let sides: Vec<_> = all.markets[0]
            .indicatives
            .iter()
            .map(|i| {
                (
                    i.side,
                    i.strikes
                        .iter()
                        .map(|s| s.strike.value())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            sides,
            vec![
                (crate::types::TakerSide::Sell, vec![1, 2]),
                (crate::types::TakerSide::Buy, vec![1, 2])
            ]
        );
    }

    #[test]
    fn empty_continuation_is_rejected() {
        let mut request = GetMyQuotesMessage::default();
        let page = MyQuotesMessage {
            request_id: request.request_id,
            quotes: Vec::new(),
            next_cursor: Some(String::new()),
        };
        assert!(!request.next_page(&page));
    }

    #[test]
    fn data_request_maps_to_one_typed_response() {
        let request_id = uuid::Uuid::new_v4();
        let request = GetActiveRfqsMessage {
            limit: crate::paging::MAX_PAGE_SIZE,
            cursor: crate::ws::types::Cursor::Start {},
            request_id,
        };
        assert!(matches!(
                request.into_message(),
                ClientMessage::GetActiveRfqs(GetActiveRfqsMessage {
        limit: crate::paging::MAX_PAGE_SIZE,
        cursor: crate::ws::types::Cursor::Start {},

                    request_id: actual
                }) if actual == request_id
            ));

        let response = ServerMessage::ActiveRfqs(ActiveRfqsData {
            next_cursor: None,

            request_id,
            rfqs: Vec::new(),
        });
        assert_eq!(
            <GetActiveRfqsMessage as MakerDataRequest>::into_response(response)
                .expect("typed response")
                .request_id,
            request_id
        );
        assert!(matches!(
            <GetActiveRfqsMessage as MakerDataRequest>::into_response(ServerMessage::RequireInvite),
            Err(boxed) if matches!(*boxed, ServerMessage::RequireInvite)
        ));
    }

    #[tokio::test]
    async fn typed_request_does_not_enqueue_an_unbound_query_after_leaving_ready() {
        let (handle, mut commands) = ManagedWsHandle::test_handle(1, 1);
        handle.inject_state(ManagedWsState::Ready {
            connection_epoch: 7,
        });
        let client = MakerDataClient {
            inner: handle.clone(),
        };

        let result = tokio::select! {
            result = client.request_all(
                LeavesReadyWhileEncoding {
                    handle,
                    request_id: uuid::Uuid::new_v4(),
                },
                Duration::from_secs(1),
            ) => result,
            _ = commands.recv() => panic!("typed data request must not enqueue after leaving Ready"),
        };

        assert!(matches!(
            result,
            Err(MakerDataRequestError::Transport(SendAwaitError::NotReady))
        ));
        assert!(matches!(commands.try_recv(), Err(TryRecvError::Empty)));
    }
    #[tokio::test]
    async fn data_client_exposes_all_owner_pushes() {
        let (handle, _commands) = ManagedWsHandle::test_handle(1, 8);
        let client = MakerDataClient {
            inner: handle.clone(),
        };
        let mut messages = client.subscribe_messages();
        for message in owner_pushes() {
            let expected = serde_json::to_value(&message).unwrap();
            handle.inject_message(message);
            let received = messages.recv().await.unwrap();
            assert_eq!(serde_json::to_value(received.message()).unwrap(), expected);
        }
    }

    fn owner_pushes() -> Vec<ServerMessage> {
        let caps = serde_json::json!({"positions": {"current": 1, "limit": 10}, "notional": [], "balances": [], "quote_liabilities": []});
        [
        serde_json::json!({"type": "MakerCapsUpdated", "data": {"caps_snapshot": caps, "reason": "lifecycle"}}),
        serde_json::json!({"type": "PositionUpdated", "data": {
            "position": {"revision": {"slot": 1, "transaction_index": 0, "instruction_index": 0}, "pda": "2PEfSGdYo17syw6vewKJceXbKHV481KWjXbo1x6g9E9D", "market_pda": "11111111111111111111111111111111", "underlying_mint": "opdhLMLuFCTUEq3YCwdtgqfuhtMqdkHBsJf6YrHfUFF", "underlying_symbol": "UNDER", "quote_mint": "Aa8SVbHR7yKKA6Zq2M2Yw9jGvNsZkMD6QKwCCCR7vgeA", "quote_symbol": "QUOTE",
                "terms": {"type": "covered_call", "strike": 1}, "side": "sell", "state": "open", "quantity": 2,
                "price": 3, "total_premium": 6, "created_at": 1, "expiry_ts": 2,
                "underlying_decimals": 9, "quote_decimals": 6,
                "underlying_price": {"type": "available", "price": 150, "published_at": 1}},
            "update_type": "created"
        }}),
        serde_json::json!({"type": "TradeExecuted", "data": {"trade": {
            "id": "00000000-0000-0000-0000-000000000001", "market_pda": "11111111111111111111111111111111", "terms": {"type": "covered_call", "strike": 1}, "side": "sell",
            "quantity": 2, "price": 3, "taker_owner":"GKwe8PT8YLdJiztcH7TpVkf7GVHFtHu4tNW9x5woDp6T", "maker_pda": "7rJSLqoFCCqN39ztKiotFqaxwxEtycG4Fm8XzF4Bvx2M", "transaction_signature": "s", "executed_at": 1
        }}}),
    ].into_iter().map(|value| serde_json::from_value(value).unwrap()).collect()
    }
}
