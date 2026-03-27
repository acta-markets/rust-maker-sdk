use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, sleep, timeout};

use crate::ws::client::WsClient;
use crate::ws::error::{WsClientError, WsResult};
use crate::ws::reconnect::{jittered_reconnect_delay, next_reconnect_delay};
use crate::ws::types::{AuthChallengeData, ClientMessage, ServerMessage};

use super::tracker::AwaitTracker;
use super::{
    ManagedCommand, ManagedWsConfig, ManagedWsEvent, SendAwaitError, normalize_maker_ws_url,
    send_event,
};

enum SessionEnd {
    CloseRequested,
    Disconnected,
}

pub(super) async fn run_managed_ws(
    config: ManagedWsConfig,
    mut cmd_rx: mpsc::Receiver<ManagedCommand>,
    messages_tx: broadcast::Sender<Arc<ServerMessage>>,
    events_tx: broadcast::Sender<ManagedWsEvent>,
) {
    let mut queued_outbound = VecDeque::<ClientMessage>::new();
    let mut tracker = AwaitTracker::new();
    let mut reconnect_delay = config.reconnect_delay;
    let mut reconnect_attempt = 0u64;
    let connect_url = normalize_maker_ws_url(&config.url);

    loop {
        match WsClient::connect(&connect_url).await {
            Ok(mut client) => {
                reconnect_attempt = 0;
                reconnect_delay = config.reconnect_delay;
                send_event(&events_tx, ManagedWsEvent::Connected);

                let outcome = run_session(
                    &mut client,
                    &config,
                    &mut cmd_rx,
                    &mut queued_outbound,
                    &mut tracker,
                    &messages_tx,
                    &events_tx,
                )
                .await;

                tracker.drain_all();

                if matches!(outcome, SessionEnd::CloseRequested) {
                    let _ = client.close().await;
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
            &mut queued_outbound,
            config.max_queued_outbound,
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
    queued_outbound: &mut VecDeque<ClientMessage>,
    tracker: &mut AwaitTracker,
    messages_tx: &broadcast::Sender<Arc<ServerMessage>>,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
) -> SessionEnd {
    if let Err(err) = authenticate(client, config, messages_tx).await {
        send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
        return SessionEnd::Disconnected;
    }
    send_event(events_tx, ManagedWsEvent::Authenticated);

    if let Some(subscribe) = &config.initial_subscribe {
        let sub = subscribe.clone();
        if let Err(err) = client.send(&ClientMessage::Subscribe(sub)).await {
            send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
            return SessionEnd::Disconnected;
        }
    }

    for message in &config.resync_messages {
        if let Err(err) = client.send(message).await {
            send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
            return SessionEnd::Disconnected;
        }
    }

    while let Some(message) = queued_outbound.pop_front() {
        if let Err(err) = client.send(&message).await {
            queued_outbound.push_front(message);
            send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
            return SessionEnd::Disconnected;
        }
    }

    let mut ping_timer = interval(config.ping_interval);
    ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_cmd = cmd_rx.recv() => {
                if let Some(end) = handle_command(maybe_cmd, client, config, queued_outbound, tracker, events_tx).await {
                    return end;
                }
            }
            _ = ping_timer.tick() => {
                if let Err(err) = client.ping().await {
                    send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                    return SessionEnd::Disconnected;
                }
            }
            read_result = read_ws(client, config.ws_read_timeout) => {
                if let Some(end) = handle_ws_read(read_result, client, config, tracker, messages_tx, events_tx).await {
                    return end;
                }
            }
        }
    }
}

async fn handle_command(
    maybe_cmd: Option<ManagedCommand>,
    client: &mut WsClient,
    config: &ManagedWsConfig,
    queued_outbound: &mut VecDeque<ClientMessage>,
    tracker: &mut AwaitTracker,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
) -> Option<SessionEnd> {
    match maybe_cmd {
        Some(ManagedCommand::Send(message)) => {
            if let Err(err) = client.send(&message).await {
                if queued_outbound.len() < config.max_queued_outbound {
                    queued_outbound.push_back(message);
                }
                send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                return Some(SessionEnd::Disconnected);
            }
        }
        Some(ManagedCommand::SendRaw(json)) => {
            if let Err(err) = client.send_text(json).await {
                send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                return Some(SessionEnd::Disconnected);
            }
        }
        Some(ManagedCommand::SendAwait {
            message,
            expected,
            request_id,
            tx,
        }) => {
            if tx.is_closed() {
                return None;
            }
            tracker.register(expected, request_id, tx);
            if let Err(err) = client.send(&message).await {
                if let Some(sender) = tracker.remove_latest() {
                    let _ = sender.send(Err(SendAwaitError::Disconnected));
                }
                send_event(events_tx, ManagedWsEvent::Error(err.to_string()));
                return Some(SessionEnd::Disconnected);
            }
        }
        Some(ManagedCommand::Close) | None => {
            return Some(SessionEnd::CloseRequested);
        }
    }
    None
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
    messages_tx: &broadcast::Sender<Arc<ServerMessage>>,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
) -> Option<SessionEnd> {
    match read_result {
        Some(Ok(server_msg)) => {
            let msg_ref = Arc::new(server_msg);
            let _ = messages_tx.send(Arc::clone(&msg_ref));
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

fn build_auth_response(
    config: &ManagedWsConfig,
    challenge: &str,
) -> Result<AuthChallengeData, String> {
    let signature = (config.challenge_signer)(challenge)
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
            let auth = build_auth_response(config, &data.challenge)?;
            client
                .auth_challenge(auth)
                .await
                .map_err(|e| e.to_string())?;
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
    messages_tx: &broadcast::Sender<Arc<ServerMessage>>,
) -> WsResult<()> {
    client.send_text(&*config.hello_json).await?;
    client.send_text(&*config.start_auth_json).await?;

    loop {
        let next = timeout(config.auth_timeout, client.next()).await;
        let message = match next {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(err))) => return Err(err),
            Ok(None) => return Err(WsClientError::ConnectionClosed),
            Err(_) => return Err(WsClientError::Timeout),
        };

        let msg_arc = Arc::new(message);
        let _ = messages_tx.send(Arc::clone(&msg_arc));

        match &*msg_arc {
            ServerMessage::AuthRequest(data) => {
                let auth = build_auth_response(config, &data.challenge)
                    .map_err(WsClientError::Protocol)?;
                client.auth_challenge(auth).await?;
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

async fn wait_reconnect_window(
    cmd_rx: &mut mpsc::Receiver<ManagedCommand>,
    queued_outbound: &mut VecDeque<ClientMessage>,
    max_queued: usize,
    delay: Duration,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
    next_attempt: u64,
) -> bool {
    send_event(
        events_tx,
        ManagedWsEvent::Reconnecting {
            attempt: next_attempt,
            delay_ms: delay.as_millis() as u64,
        },
    );

    let sleeper = sleep(delay);
    tokio::pin!(sleeper);

    loop {
        tokio::select! {
            _ = &mut sleeper => return false,
            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    Some(ManagedCommand::Send(message)) => {
                        if queued_outbound.len() < max_queued {
                            queued_outbound.push_back(message);
                        } else {
                            tracing::warn!(
                                queue_len = queued_outbound.len(),
                                "outbound queue full, dropping message during reconnect"
                            );
                        }
                    }
                    Some(ManagedCommand::SendRaw(_)) => {
                        tracing::warn!("SendRaw dropped during reconnect — raw messages cannot be queued for resend");
                    }
                    Some(ManagedCommand::SendAwait { tx, .. }) => {
                        let _ = tx.send(Err(SendAwaitError::Disconnected));
                    }
                    Some(ManagedCommand::Close) | None => return true,
                }
            }
        }
    }
}
