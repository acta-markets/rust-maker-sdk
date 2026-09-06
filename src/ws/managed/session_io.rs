use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::{Instant, timeout};

use super::AuthenticatedSessionEnd;
use super::writer::{LaneFull, WriterLanes};
use crate::ws::client::{WsClient, WsFrame, WsReader};
use crate::ws::managed::desired_subscriptions::DesiredSubscriptions;
use crate::ws::managed::tracker::AwaitTracker;
use crate::ws::managed::{ManagedCommand, ManagedWsConfig, ManagedWsError, SendAwaitError};
use crate::ws::types::ClientMessage;

/// Route a handle command onto the writer lanes. Never blocks: a full lane is
/// reported to the caller, and the socket itself is the writer task's problem.
pub(super) fn handle_command(
    maybe_cmd: Option<ManagedCommand>,
    lanes: &WriterLanes,
    tracker: &mut AwaitTracker,
    current_epoch: u64,
    subscriptions: &mut DesiredSubscriptions,
    subscription_written: &mut Option<oneshot::Receiver<Instant>>,
) -> Option<AuthenticatedSessionEnd> {
    match maybe_cmd {
        Some(ManagedCommand::Send {
            mut message,
            connection_epoch,
            tx,
        }) => {
            if tx.is_closed() {
                return None;
            }
            if connection_epoch.is_some_and(|epoch| epoch != current_epoch) {
                let _ = tx.send(Err(ManagedWsError::NotReady));
                return None;
            }
            let subscription = message.take_subscription();
            if subscription.is_some() && subscriptions.has_pending() {
                let _ = tx.send(Err(ManagedWsError::SubscriptionPending));
                return None;
            }
            let (written_tx, written_rx) =
                subscription.as_ref().map(|_| oneshot::channel()).unzip();
            match lanes.frame_with_receipt(message, Some(tx), written_tx) {
                Ok(()) => {
                    if let Some(message) = subscription {
                        subscriptions.begin(&message);
                        *subscription_written = written_rx;
                    }
                    None
                }
                Err((LaneFull::Full, done)) => {
                    if let Some(tx) = done {
                        let _ = tx.send(Err(ManagedWsError::QueueFull));
                    }
                    None
                }
                Err((LaneFull::Closed, done)) => {
                    if let Some(tx) = done {
                        let _ = tx.send(Err(ManagedWsError::Disconnected));
                    }
                    Some(AuthenticatedSessionEnd::Disconnected)
                }
            }
        }
        Some(ManagedCommand::SendAwait {
            await_id,
            mut message,
            registration,
            connection_epoch,
            tx,
        }) => {
            if tx.is_closed() {
                return None;
            }
            if connection_epoch.is_some_and(|epoch| epoch != current_epoch) {
                let _ = tx.send(Err(SendAwaitError::NotReady));
                return None;
            }
            let subscription = message.take_subscription();
            if subscription.is_some() && subscriptions.has_pending() {
                let _ = tx.send(Err(SendAwaitError::SubscriptionPending));
                return None;
            }
            if let Err((err, tx)) = tracker.register(await_id, registration, tx) {
                tracing::warn!(await_id, error = %err, "cannot register response awaiter");
                let _ = tx.send(Err(err));
                return None;
            }
            let (written_tx, written_rx) =
                subscription.as_ref().map(|_| oneshot::channel()).unzip();
            match lanes.frame_with_receipt(message, None, written_tx) {
                Ok(()) => {
                    if let Some(message) = subscription {
                        subscriptions.begin(&message);
                        *subscription_written = written_rx;
                    }
                    None
                }
                Err((LaneFull::Full, _)) => {
                    if let Some(sender) = tracker.cancel(await_id) {
                        let _ = sender.send(Err(SendAwaitError::QueueFull));
                    }
                    None
                }
                Err((LaneFull::Closed, _)) => {
                    if let Some(sender) = tracker.cancel(await_id) {
                        let _ = sender.send(Err(SendAwaitError::Disconnected));
                    }
                    Some(AuthenticatedSessionEnd::Disconnected)
                }
            }
        }
        None => Some(AuthenticatedSessionEnd::CloseRequested),
    }
}

/// Direct write on the unsplit socket, for the request/response phases before
/// the writer task exists.
pub(super) async fn write_message(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    message: &ClientMessage,
) -> Result<(), ManagedWsError> {
    match timeout(config.write_timeout, client.send(message)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(ManagedWsError::Write(err)),
        Err(_) => Err(ManagedWsError::WriteTimeout),
    }
}

pub(super) async fn read_ws(
    reader: &mut WsReader,
    ws_read_timeout: Option<Duration>,
) -> Option<crate::ws::error::WsResult<WsFrame>> {
    match ws_read_timeout {
        Some(duration) => match timeout(duration, reader.next()).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(?duration, "websocket read timed out; reconnecting");
                None
            }
        },
        None => reader.next().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::{
        client::PreparedClientMessage,
        managed::writer,
        types::{ClientMessage, GetMmSummaryMessage},
    };
    use futures_util::SinkExt;
    use tokio::sync::{broadcast, oneshot};

    #[tokio::test]
    async fn explicit_epoch_quote_is_rejected_after_queue_wait_and_reconnect() {
        use crate::ws::managed::{ManagedWsHandle, ManagedWsState};
        use crate::ws::types::QuoteMessage;
        use crate::{Nonce, OrderId, Price, QuoteExpiry, Strike};

        let (handle, mut commands) = ManagedWsHandle::test_handle(1, 1);
        handle.inject_state(ManagedWsState::Ready {
            connection_epoch: 1,
        });
        let blocking_ticket = handle.try_send(ClientMessage::Ping).unwrap();
        let quote = ClientMessage::Quote(QuoteMessage {
            rfq_id: uuid::Uuid::nil(),
            strike: Strike::new(1),
            price: Price::new(2),
            valid_until: QuoteExpiry::from_unix_seconds(100),
            nonce: Nonce::new(3),
            order_id: OrderId::new([1; 32]),
            signature: "signature".into(),
        });
        let send = handle.send_in_epoch(quote, 1);
        tokio::pin!(send);
        // Deterministically park at queue capacity, after the Ready check.
        assert!(futures_util::poll!(send.as_mut()).is_pending());
        handle.inject_state(ManagedWsState::Ready {
            connection_epoch: 2,
        });
        drop(commands.recv().await.unwrap());
        drop(blocking_ticket);
        assert!(futures_util::poll!(send.as_mut()).is_pending());
        let command = commands.recv().await.unwrap();
        assert!(matches!(
            &command,
            ManagedCommand::Send {
                connection_epoch: Some(1),
                ..
            }
        ));

        let sink = futures_util::sink::drain()
            .sink_map_err(|never| -> crate::ws::error::WsClientError { match never {} });
        let (events, _) = broadcast::channel(4);
        let (lanes, writer) = writer::spawn(sink, Duration::from_secs(1), 4, events);
        let mut tracker = AwaitTracker::new(4);
        let mut subscriptions = DesiredSubscriptions::from_initial(None);
        handle_command(
            Some(command),
            &lanes,
            &mut tracker,
            2,
            &mut subscriptions,
            &mut None,
        );
        assert!(matches!(send.await, Err(ManagedWsError::NotReady)));
        drop(lanes);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn queued_query_cannot_cross_connection_epoch() {
        let sink = futures_util::sink::drain()
            .sink_map_err(|never| -> crate::ws::error::WsClientError { match never {} });
        let (events, _) = broadcast::channel(4);
        let (lanes, writer) = writer::spawn(sink, Duration::from_secs(1), 4, events);
        let mut tracker = AwaitTracker::new(4);
        let mut subscriptions = DesiredSubscriptions::from_initial(None);
        let message = ClientMessage::GetMmSummary(GetMmSummaryMessage {
            request_id: uuid::Uuid::new_v4(),
        });
        let (tx, rx) = oneshot::channel();
        handle_command(
            Some(ManagedCommand::Send {
                message: PreparedClientMessage::new(&message).unwrap(),
                connection_epoch: Some(1),
                tx,
            }),
            &lanes,
            &mut tracker,
            2,
            &mut subscriptions,
            &mut None,
        );
        assert!(matches!(rx.await.unwrap(), Err(ManagedWsError::NotReady)));
        let (tx, rx) = oneshot::channel();
        handle_command(
            Some(ManagedCommand::SendAwait {
                await_id: 1,
                message: PreparedClientMessage::new(&message).unwrap(),
                connection_epoch: Some(1),
                registration: AwaitTracker::prepare(&message).unwrap(),
                tx,
            }),
            &lanes,
            &mut tracker,
            2,
            &mut subscriptions,
            &mut None,
        );
        assert!(matches!(rx.await.unwrap(), Err(SendAwaitError::NotReady)));
        assert_eq!(tracker.len(), 0);
        drop(lanes);
        writer.await.unwrap();
    }
}
