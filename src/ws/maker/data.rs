use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use crate::ws::managed::{
    MakerWsEndpoint, ManagedMessageReceiver, ManagedWsConfig, ManagedWsError, ManagedWsEvent,
    ManagedWsHandle, ManagedWsState, SendAwaitError, WaitUntilReadyError, spawn_managed_ws,
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
    /// Extract the typed payload.
    ///
    /// # Errors
    /// Returns the original message when its variant is not this request's
    /// response.
    fn into_response(message: ServerMessage) -> Result<Self::Response, Box<ServerMessage>>;
}

/// A typed response, owning its payload.
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
    /// Borrow the server error without cloning its payload.
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

    pub fn subscribe_messages(&self) -> ManagedMessageReceiver {
        self.inner.subscribe_messages()
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
                protocol_version: "1.0.0".to_string(),
                features: Vec::new(),
                client_name: None,
                client_version: None,
            },
            Arc::new(crate::orders::BytesSigner::from_secret([1u8; 32])),
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
}
