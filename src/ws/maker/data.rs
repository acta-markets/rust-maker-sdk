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
    /// # Errors
    /// Returns the original message when its variant is not this request's
    /// response.
    fn into_response(message: ServerMessage) -> Result<Self::Response, Box<ServerMessage>>;
}

/// A cursor-paged request. Pages run newest first by (second, id).
pub trait PagedRequest: MakerDataRequest + Clone {
    fn first_page(&mut self);
    /// Points the cursor past the rows in `so_far`; `false` once nothing is left.
    fn next_page(&mut self, so_far: &Self::Response) -> bool;
    fn append(so_far: &mut Self::Response, page: Self::Response);
}

impl PagedRequest for GetMakerPositionsMessage {
    fn first_page(&mut self) {
        self.request_id = uuid::Uuid::new_v4();
        self.limit = 500;
        self.cursor = PageCursor::Start {};
    }

    fn next_page(&mut self, so_far: &MakerPositionsMessage) -> bool {
        // the server orders a second's positions by their base58 PDA
        let last = so_far
            .positions
            .iter()
            .map(|p| (p.created_at, p.pda.to_base58(), p.pda))
            .chain(
                so_far
                    .unrenderable
                    .iter()
                    .map(|p| (p.created_at, p.pda.to_base58(), p.pda)),
            )
            .min();
        match (so_far.has_more, last) {
            (true, Some((at, _, id))) => {
                self.request_id = uuid::Uuid::new_v4();
                self.cursor = PageCursor::After { at, id };
                true
            }
            _ => false,
        }
    }

    fn append(so_far: &mut MakerPositionsMessage, page: MakerPositionsMessage) {
        so_far.positions.extend(page.positions);
        so_far.unrenderable.extend(page.unrenderable);
        so_far.has_more = page.has_more;
    }
}

impl PagedRequest for GetMyTradesMessage {
    fn first_page(&mut self) {
        self.request_id = uuid::Uuid::new_v4();
        self.limit = 200;
        self.cursor = PageCursor::Start {};
    }

    fn next_page(&mut self, so_far: &MyTradesMessage) -> bool {
        match (so_far.has_more, so_far.trades.last()) {
            (true, Some(last)) => {
                self.request_id = uuid::Uuid::new_v4();
                self.cursor = PageCursor::After {
                    at: last.confirmed_at,
                    id: last.id,
                };
                true
            }
            _ => false,
        }
    }

    fn append(so_far: &mut MyTradesMessage, page: MyTradesMessage) {
        so_far.trades.extend(page.trades);
        so_far.has_more = page.has_more;
    }
}

/// `Live` is a single snapshot; only `History` pages.
impl PagedRequest for GetMyQuotesMessage {
    fn first_page(&mut self) {
        self.request_id = uuid::Uuid::new_v4();
        if let MakerQuoteScope::History { limit, cursor } = &mut self.scope {
            *limit = 1000;
            *cursor = PageCursor::Start {};
        }
    }

    fn next_page(&mut self, so_far: &MyQuotesMessage) -> bool {
        match (&mut self.scope, so_far.has_more, so_far.quotes.last()) {
            (MakerQuoteScope::History { cursor, .. }, true, Some(last)) => {
                *cursor = PageCursor::After {
                    at: last.created_at,
                    id: last.order_id,
                };
                self.request_id = uuid::Uuid::new_v4();
                true
            }
            _ => false,
        }
    }

    fn append(so_far: &mut MyQuotesMessage, page: MyQuotesMessage) {
        so_far.quotes.extend(page.quotes);
        so_far.has_more = page.has_more;
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

    pub async fn request<R: MakerDataRequest>(
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
        let mut all = self.request(request.clone(), timeout).await?.into_payload();
        while request.next_page(&all) {
            let page = self.request(request.clone(), timeout).await?.into_payload();
            R::append(&mut all, page);
        }
        Ok(all)
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

macro_rules! data_request {
    ($request:ty, $client_variant:ident, $response:ty, $server_variant:ident) => {
        impl private::Sealed for $request {}

        impl MakerDataRequest for $request {
            type Response = $response;

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
    ActiveRfqs
);
data_request!(GetMarketsMessage, GetMarkets, MarketsData, Markets);
data_request!(
    GetMarketDescriptorsMessage,
    GetMarketDescriptors,
    MarketDescriptorsData,
    MarketDescriptors
);
data_request!(GetExpiriesMessage, GetExpiries, ExpiriesData, Expiries);
data_request!(GetTokensMessage, GetTokens, TokensData, Tokens);
data_request!(
    GetIndicativePricesMessage,
    GetIndicativePrices,
    IndicativePricesMessage,
    IndicativePrices
);
data_request!(
    GetMakerPositionsMessage,
    GetMakerPositions,
    MakerPositionsMessage,
    MakerPositions
);
data_request!(GetMyQuotesMessage, GetMyQuotes, MyQuotesMessage, MyQuotes);
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
    MakerMarkets
);
data_request!(GetTokenCapsMessage, GetTokenCaps, TokenCapsData, TokenCaps);
data_request!(GetMyCapsMessage, GetMyCaps, MyCapsData, MyCaps);
data_request!(CheckQuoteMessage, CheckQuote, QuoteCheckData, QuoteCheck);
data_request!(GetMyTradesMessage, GetMyTrades, MyTradesMessage, MyTrades);
data_request!(
    GetEarnSummaryMessage,
    GetEarnSummary,
    EarnSummaryData,
    EarnSummary
);
data_request!(GetMmSummaryMessage, GetMmSummary, MmSummaryData, MmSummary);
data_request!(
    GetTokenMarketsInfoMessage,
    GetTokenMarketsInfo,
    TokenMarketsInfoData,
    TokenMarketsInfo
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
    fn trade_pages_continue_after_the_last_row_until_has_more_is_false() {
        let page = |id: u128, confirmed_at: u64, has_more: bool| -> MyTradesMessage {
            serde_json::from_value(serde_json::json!({
                "request_id": uuid::Uuid::nil(),
                "trades": [{
                    "id": uuid::Uuid::from_u128(id), "rfq_id": uuid::Uuid::nil(),
                    "market_pda": crate::Address32::new([1; 32]),
                    "underlying_mint": crate::Address32::new([2; 32]), "underlying_symbol": "SOL",
                    "underlying_decimals": 9,
                    "quote_mint": crate::Address32::new([3; 32]), "quote_symbol": "USDC",
                    "quote_decimals": 6,
                    "terms": {"type": "covered_call", "strike": 100}, "side": "sell",
                    "taker": crate::Address32::new([4; 32]), "quantity": 1, "price": 1,
                    "tx_signature": "sig", "position_pda": null, "confirmed_at": confirmed_at
                }],
                "has_more": has_more
            }))
            .expect("trades page")
        };
        let mut request = GetMyTradesMessage {
            request_id: uuid::Uuid::nil(),
            limit: 1,
            cursor: PageCursor::After {
                at: std::time::UNIX_EPOCH,
                id: uuid::Uuid::nil(),
            },
            market: QueryFilter::All {},
        };
        request.first_page();
        assert_eq!(
            (request.limit, &request.cursor),
            (200, &PageCursor::Start {})
        );

        let mut so_far = page(2, 20, true);
        assert!(request.next_page(&so_far));
        assert_eq!(
            request.cursor,
            PageCursor::After {
                at: std::time::UNIX_EPOCH + Duration::from_secs(20),
                id: uuid::Uuid::from_u128(2),
            }
        );
        GetMyTradesMessage::append(&mut so_far, page(1, 10, false));
        assert_eq!(so_far.trades.len(), 2);
        assert!(!request.next_page(&so_far));
    }

    #[test]
    fn live_quotes_are_one_page() {
        let mut request = GetMyQuotesMessage::default();
        request.first_page();
        assert_eq!(request.scope, MakerQuoteScope::Live {});
        let so_far = MyQuotesMessage {
            request_id: request.request_id,
            quotes: Vec::new(),
            has_more: true,
        };
        assert!(!request.next_page(&so_far));
    }

    #[test]
    fn data_request_maps_to_one_typed_response() {
        let request_id = uuid::Uuid::new_v4();
        let request = GetActiveRfqsMessage { request_id };
        assert!(matches!(
            request.into_message(),
            ClientMessage::GetActiveRfqs(GetActiveRfqsMessage {
                request_id: actual
            }) if actual == request_id
        ));

        let response = ServerMessage::ActiveRfqs(ActiveRfqsData {
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
            result = client.request(
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
            "position": {"revision": {"slot": 1, "transaction_index": 0, "instruction_index": 0}, "pda": "2PEfSGdYo17syw6vewKJceXbKHV481KWjXbo1x6g9E9D", "market": "11111111111111111111111111111111", "underlying_mint": "opdhLMLuFCTUEq3YCwdtgqfuhtMqdkHBsJf6YrHfUFF", "quote_mint": "Aa8SVbHR7yKKA6Zq2M2Yw9jGvNsZkMD6QKwCCCR7vgeA",
                "terms": {"type": "covered_call", "strike": 1}, "side": "sell", "state": "open", "quantity": 2,
                "price": 3, "total_premium": 6, "created_at": 1, "expiry_ts": 2,
                "underlying_decimals": 9, "quote_decimals": 6},
            "update_type": "created"
        }}),
        serde_json::json!({"type": "TradeExecuted", "data": {"trade": {
            "id": "00000000-0000-0000-0000-000000000001", "market": "11111111111111111111111111111111", "terms": {"type": "covered_call", "strike": 1}, "side": "sell",
            "quantity": 2, "price": 3, "taker": "GKwe8PT8YLdJiztcH7TpVkf7GVHFtHu4tNW9x5woDp6T", "maker": "7rJSLqoFCCqN39ztKiotFqaxwxEtycG4Fm8XzF4Bvx2M", "tx_signature": "s", "executed_at": 1
        }}}),
    ].into_iter().map(|value| serde_json::from_value(value).unwrap()).collect()
    }
}
