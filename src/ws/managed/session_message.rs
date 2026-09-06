use std::sync::Arc;

use tokio::sync::{broadcast, watch};
use tokio::time::timeout;

use super::auth;
use super::writer::WriterLanes;
use super::{
    AuthenticatedSessionEnd, AwaitTracker, InboundPublisher, ManagedWsConfig, ManagedWsEvent,
    SessionFailure, send_event,
};
use crate::ws::client::PreparedClientMessage;
use crate::ws::types::{ClientMessage, ServerMessage};

pub(super) async fn handle_ws_read(
    read_result: Option<crate::ws::error::WsResult<ServerMessage>>,
    lanes: &WriterLanes,
    config: &ManagedWsConfig,
    tracker: &mut AwaitTracker,
    inbound: &mut InboundPublisher<'_>,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Option<AuthenticatedSessionEnd> {
    match read_result {
        Some(Ok(server_msg)) => {
            let message = inbound.publish(server_msg);
            if let Some(sender) = tracker.take_for_message(&message) {
                let _ = sender.send(Ok(Arc::clone(&message)));
            }
            if let Err(error) =
                handle_session_message(lanes, config, &message, events_tx, shutdown_rx).await
            {
                if !matches!(error, SessionFailure::Shutdown) {
                    send_event(events_tx, ManagedWsEvent::Error(error.to_string()));
                }
                return Some(error.into_session_end());
            }
        }
        Some(Err(error)) => {
            send_event(events_tx, ManagedWsEvent::Error(error.to_string()));
            return Some(AuthenticatedSessionEnd::Disconnected);
        }
        None => return Some(AuthenticatedSessionEnd::Disconnected),
    }
    None
}

async fn handle_session_message(
    lanes: &WriterLanes,
    config: &ManagedWsConfig,
    message: &ServerMessage,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<(), SessionFailure> {
    match message {
        ServerMessage::AuthRequest(data) => {
            let signing = timeout(
                config.auth_timeout,
                auth::build_auth_response(config, &data.challenge),
            );
            let signed = tokio::select! {
                biased;
                _ = wait_for_shutdown(shutdown_rx) => return Err(SessionFailure::Shutdown),
                result = signing => result,
            };
            let auth = signed
                .map_err(|_| SessionFailure::Retryable("challenge signing timed out".to_string()))?
                .map_err(|error| {
                    SessionFailure::Retryable(format!("challenge signer failed: {error}"))
                })?;
            let frame = PreparedClientMessage::new(&ClientMessage::AuthChallenge(auth))
                .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
            lanes.frame(frame, None).map_err(|_| {
                SessionFailure::Retryable("auth response could not be queued".to_string())
            })?;
        }
        ServerMessage::AuthSuccess(_) => {
            send_event(events_tx, ManagedWsEvent::Authenticated);
        }
        ServerMessage::RequestError(envelope) => {
            tracing::error!(
                request_id = %envelope.request_id,
                error = ?envelope.error,
                "request error"
            );
        }
        ServerMessage::SubscribeAck(ack) => {
            tracing::info!(
                request_id = %ack.request_id,
                subscribed = ?ack.subscribed,
                "subscribe ack"
            );
        }
        ServerMessage::UnsubscribeAck(ack) => {
            tracing::info!(
                request_id = %ack.request_id,
                unsubscribed = ?ack.unsubscribed,
                "unsubscribe ack"
            );
        }
        ServerMessage::SubscriptionUpdated(data) => {
            tracing::info!(
                request_id = %data.request_id,
                channels = ?data.channels,
                "subscription updated"
            );
        }
        _ => {}
    }
    if let Some(failure) = SessionFailure::from_control(message) {
        return Err(failure);
    }
    Ok(())
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    if *shutdown_rx.borrow() {
        return;
    }
    let _ = shutdown_rx.changed().await;
}
