use crate::ws::managed::{
    MakerWsEndpoint, ManagedMessageReceiver, ManagedWsConfig, ManagedWsError, ManagedWsEvent,
    ManagedWsHandle, ManagedWsState, ReferenceChangeReceiver, SendTicket, WaitUntilReadyError,
    spawn_managed_ws,
};
use crate::ws::types::{
    AddChannelsData, AddMintsData, BatchQuotesMessage, CancelAllQuotesMessage, CancelQuoteData,
    ClientMessage, IndicativePricesResponseMessage, QuoteMessage, RemoveChannelsData,
    RemoveMintsData, ReplaceQuoteMessage, SubscribeData, UnsubscribeData,
};
use tokio::sync::broadcast;

use super::MakerClientError;

/// Maker quote-plane client.
///
/// `try_*` methods return immediately when the outbound queue is full. A
/// successful [`SendTicket`] reports a socket write, not a server acknowledgement.
#[derive(Clone)]
pub struct MakerQuoteClient {
    inner: ManagedWsHandle,
}

impl MakerQuoteClient {
    pub fn spawn(config: ManagedWsConfig) -> Result<Self, MakerClientError> {
        if config.endpoint != MakerWsEndpoint::Quote {
            return Err(MakerClientError::WrongEndpoint {
                client: "MakerQuoteClient",
                required: MakerWsEndpoint::Quote,
                actual: config.endpoint,
            });
        }
        Ok(Self {
            inner: spawn_managed_ws(config)?,
        })
    }

    /// Send only in `connection_epoch`, the epoch whose recovery the strategy
    /// applied; a reconnect in between rejects the quote with `NotReady`.
    /// Success confirms only the socket write. Subscribe before sending and feed
    /// messages to [`super::QuoteOutcomeTracker`] to track the server outcome.
    pub async fn quote_in_epoch(
        &self,
        payload: QuoteMessage,
        connection_epoch: u64,
    ) -> Result<(), ManagedWsError> {
        self.inner
            .send_in_epoch(ClientMessage::Quote(payload), connection_epoch)
            .await
    }

    pub fn try_quote_in_epoch(
        &self,
        payload: QuoteMessage,
        connection_epoch: u64,
    ) -> Result<SendTicket, ManagedWsError> {
        self.inner
            .try_send_in_epoch(ClientMessage::Quote(payload), connection_epoch)
    }

    pub async fn replace_in_epoch(
        &self,
        payload: ReplaceQuoteMessage,
        connection_epoch: u64,
    ) -> Result<(), ManagedWsError> {
        self.inner
            .send_in_epoch(ClientMessage::ReplaceQuote(payload), connection_epoch)
            .await
    }

    pub fn try_replace_in_epoch(
        &self,
        payload: ReplaceQuoteMessage,
        connection_epoch: u64,
    ) -> Result<SendTicket, ManagedWsError> {
        self.inner
            .try_send_in_epoch(ClientMessage::ReplaceQuote(payload), connection_epoch)
    }

    pub async fn batch_quotes_in_epoch(
        &self,
        payload: BatchQuotesMessage,
        connection_epoch: u64,
    ) -> Result<(), ManagedWsError> {
        self.inner
            .send_in_epoch(ClientMessage::BatchQuotes(payload), connection_epoch)
            .await
    }

    pub fn try_batch_quotes_in_epoch(
        &self,
        payload: BatchQuotesMessage,
        connection_epoch: u64,
    ) -> Result<SendTicket, ManagedWsError> {
        self.inner
            .try_send_in_epoch(ClientMessage::BatchQuotes(payload), connection_epoch)
    }

    pub async fn cancel_quote(&self, payload: CancelQuoteData) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.send(ClientMessage::CancelQuote(payload)).await
    }

    pub fn try_cancel_quote(&self, payload: CancelQuoteData) -> Result<SendTicket, ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.try_send(ClientMessage::CancelQuote(payload))
    }

    pub async fn cancel_all_quotes(
        &self,
        payload: CancelAllQuotesMessage,
    ) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner
            .send(ClientMessage::CancelAllQuotes(payload))
            .await
    }

    pub fn try_cancel_all_quotes(
        &self,
        payload: CancelAllQuotesMessage,
    ) -> Result<SendTicket, ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.try_send(ClientMessage::CancelAllQuotes(payload))
    }

    pub async fn indicative_prices_response(
        &self,
        payload: IndicativePricesResponseMessage,
    ) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner
            .send(ClientMessage::IndicativePricesResponse(payload))
            .await
    }

    pub fn try_indicative_prices_response(
        &self,
        payload: IndicativePricesResponseMessage,
    ) -> Result<SendTicket, ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner
            .try_send(ClientMessage::IndicativePricesResponse(payload))
    }

    pub async fn subscribe(&self, payload: SubscribeData) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.send(ClientMessage::Subscribe(payload)).await
    }

    pub async fn unsubscribe(&self, payload: UnsubscribeData) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.send(ClientMessage::Unsubscribe(payload)).await
    }

    pub async fn add_mints(&self, payload: AddMintsData) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.send(ClientMessage::AddMints(payload)).await
    }

    pub async fn remove_mints(&self, payload: RemoveMintsData) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.send(ClientMessage::RemoveMints(payload)).await
    }

    pub async fn add_channels(&self, payload: AddChannelsData) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.send(ClientMessage::AddChannels(payload)).await
    }

    pub async fn remove_channels(&self, payload: RemoveChannelsData) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner
            .send(ClientMessage::RemoveChannels(payload))
            .await
    }

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ws::types::HelloData;

    #[test]
    fn quote_client_rejects_data_endpoint_before_spawning() {
        let config = ManagedWsConfig::new(
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
        .with_endpoint(MakerWsEndpoint::Data);

        assert!(matches!(
            MakerQuoteClient::spawn(config),
            Err(MakerClientError::WrongEndpoint {
                required: MakerWsEndpoint::Quote,
                actual: MakerWsEndpoint::Data,
                ..
            })
        ));
    }
}
