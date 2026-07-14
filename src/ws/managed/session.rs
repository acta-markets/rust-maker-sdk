use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::time::{Instant, interval, timeout};

use crate::ws::client::WsClient;
use crate::ws::error::{WsClientError, WsResult};
use crate::ws::reconnect::{jittered_reconnect_delay, next_reconnect_delay};
use uuid::Uuid;

use crate::ws::types::{
    AuthChallengeData, ClientMessage, GetActiveRfqsMessage, GetMmSummaryMessage, ServerMessage,
};

use super::reconnect_window::wait_reconnect_window;
use super::tracker::AwaitTracker;
use super::{
    ManagedCommand, ManagedInbound, ManagedWsConfig, ManagedWsEvent, SendAwaitError,
    normalize_maker_ws_url_for_endpoint, send_event,
};

enum SessionEnd {
    CloseRequested,
    Disconnected,
}

struct InboundPublisher<'a> {
    tx: &'a broadcast::Sender<Arc<ManagedInbound>>,
    connection_epoch: u64,
    sequence: &'a mut u64,
}

impl InboundPublisher<'_> {
    fn publish(&mut self, message: ServerMessage) -> Arc<ServerMessage> {
        *self.sequence = self.sequence.wrapping_add(1);
        let message = Arc::new(message);
        let inbound = ManagedInbound {
            connection_epoch: self.connection_epoch,
            sequence: *self.sequence,
            received_at: std::time::Instant::now(),
            message: Arc::clone(&message),
        };
        let _ = self.tx.send(Arc::new(inbound));
        message
    }
}

pub(super) async fn run_managed_ws(
    config: ManagedWsConfig,
    mut cmd_rx: mpsc::Receiver<ManagedCommand>,
    mut cancel_rx: mpsc::UnboundedReceiver<u64>,
    messages_tx: broadcast::Sender<Arc<ManagedInbound>>,
    events_tx: broadcast::Sender<ManagedWsEvent>,
) {
    let mut tracker = AwaitTracker::new(config.max_pending_awaits);
    let mut reconnect_delay = config.reconnect_delay;
    let mut reconnect_attempt = 0u64;
    let mut connection_epoch = 0u64;
    let connect_url = normalize_maker_ws_url_for_endpoint(&config.url, config.endpoint);

    loop {
        match WsClient::connect_with_config(&connect_url, config.transport).await {
            Ok(mut client) => {
                connection_epoch = connection_epoch.wrapping_add(1);
                let mut inbound_sequence = 0u64;
                let mut inbound = InboundPublisher {
                    tx: &messages_tx,
                    connection_epoch,
                    sequence: &mut inbound_sequence,
                };
                reconnect_attempt = 0;
                reconnect_delay = config.reconnect_delay;
                send_event(&events_tx, ManagedWsEvent::Connected);

                let outcome = run_session(
                    &mut client,
                    &config,
                    &mut cmd_rx,
                    &mut cancel_rx,
                    &mut tracker,
                    &mut inbound,
                    &events_tx,
                )
                .await;

                tracker.drain_all();

                if matches!(outcome, SessionEnd::CloseRequested) {
                    let _ = timeout(config.write_timeout, client.close()).await;
                    return;
                }
                send_event(&events_tx, ManagedWsEvent::Disconnected);
            }
            Err(err) => {
                send_event(&events_tx, ManagedWsEvent::Error(err.to_string()));
            }
        }

        if wait_reconnect_window(
            &mut cmd_rx,
            &mut cancel_rx,
            jittered_reconnect_delay(reconnect_delay),
            &events_tx,
            reconnect_attempt + 1,
        )
        .await
        {
            return;
        }

        reconnect_attempt += 1;
        reconnect_delay = next_reconnect_delay(reconnect_delay, config.max_reconnect_delay);
    }
}

async fn run_session(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    cmd_rx: &mut mpsc::Receiver<ManagedCommand>,
    cancel_rx: &mut mpsc::UnboundedReceiver<u64>,
    tracker: &mut AwaitTracker,
    inbound: &mut InboundPublisher<'_>,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
) -> SessionEnd {
    match timeout(config.auth_timeout, authenticate(client, config, inbound)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
            return SessionEnd::Disconnected;
        }
        Err(_) => {
            send_event(
                events_tx,
                ManagedWsEvent::Error("authentication timed out".to_string()),
            );
            return SessionEnd::Disconnected;
        }
    }
    send_event(events_tx, ManagedWsEvent::Authenticated);

    if let Some(subscribe) = &config.initial_subscribe {
        let sub = subscribe.clone();
        if let Err(err) = write_message(client, config, &ClientMessage::Subscribe(sub)).await {
            send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
            return SessionEnd::Disconnected;
        }
    }

    if config.auto_reconcile {
        let reconcile = [
            ClientMessage::GetMmSummary(GetMmSummaryMessage {
                request_id: Uuid::new_v4(),
            }),
            ClientMessage::GetActiveRfqs(GetActiveRfqsMessage {
                request_id: Uuid::new_v4(),
            }),
        ];
        for message in reconcile {
            if let Err(err) = write_message(client, config, &message).await {
                send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                return SessionEnd::Disconnected;
            }
        }
    }

    let mut ping_timer = interval(config.ping_interval);
    ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pong_deadline = None;

    loop {
        tokio::select! {
            maybe_cmd = cmd_rx.recv() => {
                if let Some(end) = handle_command(maybe_cmd, client, config, tracker, events_tx).await {
                    return end;
                }
            }
            maybe_await_id = cancel_rx.recv() => {
                if let Some(await_id) = maybe_await_id {
                    tracker.cancel(await_id);
                }
            }
            _ = ping_timer.tick(), if pong_deadline.is_none() => {
                match timeout(config.write_timeout, client.ping()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                        return SessionEnd::Disconnected;
                    }
                    Err(_) => {
                        send_event(events_tx, ManagedWsEvent::Error("ping write timed out".to_string()));
                        return SessionEnd::Disconnected;
                    }
                }
                pong_deadline = Some(Instant::now() + config.pong_timeout);
            }
            _ = wait_for_deadline(pong_deadline), if pong_deadline.is_some() => {
                send_event(events_tx, ManagedWsEvent::Error("pong deadline exceeded".to_string()));
                return SessionEnd::Disconnected;
            }
            read_result = read_ws(client, config.ws_read_timeout) => {
                if matches!(read_result, Some(Ok(ServerMessage::Pong(_)))) {
                    pong_deadline = None;
                }
                if let Some(end) = handle_ws_read(
                    read_result,
                    client,
                    config,
                    tracker,
                    inbound,
                    events_tx,
                ).await {
                    return end;
                }
            }
        }
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    } else {
        std::future::pending::<()>().await;
    }
}

async fn handle_command(
    maybe_cmd: Option<ManagedCommand>,
    client: &mut WsClient,
    config: &ManagedWsConfig,
    tracker: &mut AwaitTracker,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
) -> Option<SessionEnd> {
    match maybe_cmd {
        Some(ManagedCommand::Send { message, tx }) => {
            if tx.is_closed() {
                return None;
            }
            if let Err(err) = write_message(client, config, &message).await {
                send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                let _ = tx.send(Err(err));
                return Some(SessionEnd::Disconnected);
            }
            let _ = tx.send(Ok(()));
        }
        Some(ManagedCommand::SendAwait {
            await_id,
            message,
            tx,
        }) => {
            if tx.is_closed() {
                return None;
            }
            if let Err((err, tx)) = tracker.register(await_id, &message, tx) {
                tracing::warn!(await_id, error = %err, "cannot register response awaiter");
                let _ = tx.send(Err(err));
                return None;
            }
            if let Err(err) = write_message(client, config, &message).await {
                if let Some(sender) = tracker.cancel(await_id) {
                    let _ = sender.send(Err(SendAwaitError::Disconnected));
                }
                send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                return Some(SessionEnd::Disconnected);
            }
        }
        Some(ManagedCommand::Close { tx }) => {
            let _ = tx.send(());
            return Some(SessionEnd::CloseRequested);
        }
        None => {
            return Some(SessionEnd::CloseRequested);
        }
    }
    None
}

async fn write_message(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    message: &ClientMessage,
) -> Result<(), super::ManagedWsError> {
    match timeout(config.write_timeout, client.send(message)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(super::ManagedWsError::Write(err)),
        Err(_) => Err(super::ManagedWsError::WriteTimeout),
    }
}

async fn read_ws(
    client: &mut WsClient,
    ws_read_timeout: Option<Duration>,
) -> Option<crate::ws::error::WsResult<ServerMessage>> {
    match ws_read_timeout {
        Some(dur) => match tokio::time::timeout(dur, client.next()).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!("ws read timeout after {:?}, reconnecting", dur);
                None
            }
        },
        None => client.next().await,
    }
}

async fn handle_ws_read(
    read_result: Option<crate::ws::error::WsResult<ServerMessage>>,
    client: &mut WsClient,
    config: &ManagedWsConfig,
    tracker: &mut AwaitTracker,
    inbound: &mut InboundPublisher<'_>,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
) -> Option<SessionEnd> {
    match read_result {
        Some(Ok(server_msg)) => {
            let msg_ref = inbound.publish(server_msg);
            if let Some(sender) = tracker.take_for_message(&msg_ref) {
                let _ = sender.send(Ok(Arc::clone(&msg_ref)));
            }
            if let Err(err) = handle_session_message(client, config, &msg_ref, events_tx).await {
                send_event(events_tx, ManagedWsEvent::Error(err));
                return Some(SessionEnd::Disconnected);
            }
        }
        Some(Err(err)) => {
            send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
            return Some(SessionEnd::Disconnected);
        }
        None => return Some(SessionEnd::Disconnected),
    }
    None
}

async fn build_auth_response(
    config: &ManagedWsConfig,
    challenge: &str,
) -> Result<AuthChallengeData, String> {
    let signature = config
        .challenge_signing
        .sign(challenge)
        .await
        .map_err(|err| format!("challenge signer failed: {err}"))?;
    Ok(AuthChallengeData {
        challenge: challenge.to_owned(),
        signature,
        pubkey: config.auth_pubkey.clone(),
    })
}

fn format_auth_error_message(reason: &str, message: Option<&str>) -> String {
    match message {
        Some(message) => format!("{reason} ({message})"),
        None => reason.to_string(),
    }
}

async fn handle_session_message(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    msg: &ServerMessage,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
) -> Result<(), String> {
    match msg {
        ServerMessage::AuthRequest(data) => {
            let auth = timeout(
                config.auth_timeout,
                build_auth_response(config, &data.challenge),
            )
            .await
            .map_err(|_| "challenge signing timed out".to_string())??;
            timeout(config.write_timeout, client.auth_challenge(auth))
                .await
                .map_err(|_| "auth response write timed out".to_string())?
                .map_err(|error| error.to_string())?;
        }
        ServerMessage::AuthSuccess(_) => {
            send_event(events_tx, ManagedWsEvent::Authenticated);
        }
        ServerMessage::AuthError(err) => {
            return Err(format!(
                "auth error: {}",
                format_auth_error_message(err.reason.as_str(), err.message.as_deref())
            ));
        }
        ServerMessage::RequestError(env) => {
            tracing::error!(
                request_id = %env.request_id,
                "request error: {:?}", env.error
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
    Ok(())
}

async fn authenticate(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
) -> WsResult<()> {
    timeout(config.write_timeout, client.send_text(&*config.hello_json))
        .await
        .map_err(|_| WsClientError::Timeout)??;
    timeout(
        config.write_timeout,
        client.send_text(&*config.start_auth_json),
    )
    .await
    .map_err(|_| WsClientError::Timeout)??;

    loop {
        let message = match client.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(err)) => return Err(err),
            None => return Err(WsClientError::ConnectionClosed),
        };

        let msg_arc = inbound.publish(message);

        match &*msg_arc {
            ServerMessage::AuthRequest(data) => {
                let auth = build_auth_response(config, &data.challenge)
                    .await
                    .map_err(WsClientError::Protocol)?;
                timeout(config.write_timeout, client.auth_challenge(auth))
                    .await
                    .map_err(|_| WsClientError::Timeout)??;
            }
            ServerMessage::AuthSuccess(_) => {
                return Ok(());
            }
            ServerMessage::AuthError(err) => {
                return Err(WsClientError::Protocol(format!(
                    "auth error: {}",
                    format_auth_error_message(err.reason.as_str(), err.message.as_deref())
                )));
            }
            _ => {}
        }
    }
}
