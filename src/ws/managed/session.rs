use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::{Instant, interval, timeout};

use crate::ws::client::{PreparedClientMessage, WsClient, WsFrame};
use crate::ws::reconnect::{ReconnectBackoff, jittered_reconnect_delay_with_ratio};
use crate::ws::types::{ClientMessage, ServerMessage};

use super::desired_subscriptions::DesiredSubscriptions;
use super::reconnect_window::wait_reconnect_window;
use super::tracker::AwaitTracker;
use super::writer::{self, LaneFull};
use super::{
    ManagedCommand, ManagedInbound, ManagedWsConfig, ManagedWsError, ManagedWsEvent,
    ManagedWsState, ManagedWsTerminationReason, normalize_maker_ws_url_for_endpoint, send_event,
    transition_state,
};

const INVALID_SIGNATURE_STRIKE_LIMIT: u32 = 3;

enum SessionEnd {
    CloseRequested,
    AuthenticationFailed,
    Terminated(ManagedWsTerminationReason),
    Disconnected {
        authenticated_for: Duration,
        clear_resume: bool,
    },
}

enum AuthenticatedSessionEnd {
    CloseRequested,
    Disconnected,
    CredentialsExpired,
    Terminated(ManagedWsTerminationReason),
}

impl AuthenticatedSessionEnd {
    fn finish(self, authenticated_at: Instant) -> SessionEnd {
        match self {
            Self::CloseRequested => SessionEnd::CloseRequested,
            Self::Disconnected => SessionEnd::Disconnected {
                authenticated_for: authenticated_at.elapsed(),
                clear_resume: false,
            },
            Self::CredentialsExpired => SessionEnd::Disconnected {
                authenticated_for: authenticated_at.elapsed(),
                clear_resume: true,
            },
            Self::Terminated(reason) => SessionEnd::Terminated(reason),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum SessionFailure {
    #[error("{0}")]
    Retryable(String),
    #[error("session was replaced by another authenticated connection")]
    CredentialsExpired(String),
    #[error("managed websocket terminated: {0:?}")]
    Terminated(ManagedWsTerminationReason),
    #[error("shutdown requested")]
    Shutdown,
}

impl SessionFailure {
    fn from_control(message: &ServerMessage) -> Option<Self> {
        control::classify(message).map(|failure| match failure {
            control::ControlFailure::Retry(detail) => Self::Retryable(detail),
            control::ControlFailure::ClearCredentials(detail) => Self::CredentialsExpired(detail),
            control::ControlFailure::Terminate(reason) => Self::Terminated(reason),
        })
    }

    fn into_session_end(self) -> AuthenticatedSessionEnd {
        match self {
            Self::CredentialsExpired(_) => AuthenticatedSessionEnd::CredentialsExpired,
            Self::Terminated(reason) => AuthenticatedSessionEnd::Terminated(reason),
            Self::Retryable(_) => AuthenticatedSessionEnd::Disconnected,
            Self::Shutdown => AuthenticatedSessionEnd::CloseRequested,
        }
    }
}

#[path = "session_auth.rs"]
mod auth;
#[path = "session_control.rs"]
mod control;
#[path = "session_io.rs"]
mod io;
#[path = "session_ready.rs"]
mod ready;
#[path = "session_message.rs"]
mod session_message;

struct InboundPublisher<'a> {
    tx: &'a broadcast::Sender<ManagedInbound>,
    connection_epoch: u64,
    sequence: &'a mut u64,
}

struct SessionRuntime<'a, 'inbound> {
    tracker: &'a mut AwaitTracker,
    subscriptions: &'a mut DesiredSubscriptions,
    inbound: &'a mut InboundPublisher<'inbound>,
    gap_rx: &'a mut mpsc::UnboundedReceiver<()>,
    events_tx: &'a broadcast::Sender<ManagedWsEvent>,
    resume_session_id: &'a mut Option<String>,
    shutdown_rx: &'a mut watch::Receiver<bool>,
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
        let _ = self.tx.send(inbound);
        message
    }
}

/// Receiving ends of the handle's channels.
pub(super) struct HandleInputs {
    pub(super) cmd_rx: mpsc::Receiver<ManagedCommand>,
    pub(super) cancel_rx: mpsc::UnboundedReceiver<u64>,
    pub(super) gap_rx: mpsc::UnboundedReceiver<()>,
}

pub(super) async fn run_managed_ws(
    config: ManagedWsConfig,
    inputs: HandleInputs,
    messages_tx: broadcast::Sender<ManagedInbound>,
    events_tx: broadcast::Sender<ManagedWsEvent>,
    state_tx: watch::Sender<ManagedWsState>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let HandleInputs {
        mut cmd_rx,
        mut cancel_rx,
        mut gap_rx,
    } = inputs;
    let mut tracker = AwaitTracker::new(config.max_pending_awaits);
    let mut subscriptions = DesiredSubscriptions::from_initial(config.initial_subscribe.as_ref());
    let mut backoff = ReconnectBackoff::new(config.reconnect_delay, config.max_reconnect_delay);
    let mut connection_epoch = 0u64;
    let mut resume_session_id = None;
    let mut invalid_signature_strikes = 0u32;
    let mut pending_auth_rejection: Option<ManagedWsTerminationReason> = None;
    let connect_url = normalize_maker_ws_url_for_endpoint(&config.url, config.endpoint);

    loop {
        let connect_result = tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown_rx) => {
                terminate(&events_tx, &state_tx, ManagedWsTerminationReason::Requested);
                return;
            }
            result = WsClient::connect_with_config(&connect_url, config.effective_transport()) => result,
        };
        match connect_result {
            Ok(client) => {
                connection_epoch = connection_epoch.wrapping_add(1);
                let mut inbound_sequence = 0u64;
                let mut inbound = InboundPublisher {
                    tx: &messages_tx,
                    connection_epoch,
                    sequence: &mut inbound_sequence,
                };
                transition_state(
                    &state_tx,
                    &events_tx,
                    ManagedWsState::Connected { connection_epoch },
                    ManagedWsEvent::Connected,
                );

                let outcome = run_session(
                    client,
                    &config,
                    &mut cmd_rx,
                    &mut cancel_rx,
                    SessionRuntime {
                        tracker: &mut tracker,
                        subscriptions: &mut subscriptions,
                        inbound: &mut inbound,
                        gap_rx: &mut gap_rx,
                        events_tx: &events_tx,
                        resume_session_id: &mut resume_session_id,
                        shutdown_rx: &mut shutdown_rx,
                    },
                    &state_tx,
                    connection_epoch,
                )
                .await;

                tracker.drain_all();
                subscriptions.connection_lost();

                match outcome {
                    SessionEnd::CloseRequested => {
                        terminate(&events_tx, &state_tx, ManagedWsTerminationReason::Requested);
                        return;
                    }
                    SessionEnd::AuthenticationFailed => {}
                    SessionEnd::Terminated(reason) => {
                        // A lone invalid_signature can be a signer or server
                        // glitch; only a streak proves bad credentials.
                        let strike = matches!(
                            &reason,
                            ManagedWsTerminationReason::AuthenticationRejected {
                                reason,
                                ..
                            } if reason == "invalid_signature"
                        );
                        if strike {
                            invalid_signature_strikes += 1;
                        }
                        if strike && invalid_signature_strikes < INVALID_SIGNATURE_STRIKE_LIMIT {
                            pending_auth_rejection = Some(reason);
                            send_event(
                                &events_tx,
                                ManagedWsEvent::Error(format!(
                                    "authentication rejected with invalid_signature, retrying \
                                     ({invalid_signature_strikes}/{INVALID_SIGNATURE_STRIKE_LIMIT})"
                                )),
                            );
                        } else {
                            terminate(&events_tx, &state_tx, reason);
                            return;
                        }
                    }
                    SessionEnd::Disconnected {
                        authenticated_for,
                        clear_resume,
                    } => {
                        invalid_signature_strikes = 0;
                        pending_auth_rejection = None;
                        if clear_resume {
                            resume_session_id = None;
                        }
                        let reset = backoff.reset_after_stable_session(authenticated_for);
                        tracing::debug!(
                            authenticated_for_ms = authenticated_for.as_millis(),
                            reset,
                            "authenticated websocket session ended"
                        );
                    }
                }
                send_event(&events_tx, ManagedWsEvent::Disconnected);
            }
            Err(err) => {
                send_event(&events_tx, ManagedWsEvent::Error(err.to_string()));
            }
        }

        if backoff.limit_reached(config.max_reconnect_attempts) {
            send_event(
                &events_tx,
                ManagedWsEvent::Error(format!(
                    "reconnect attempt limit reached ({})",
                    config.max_reconnect_attempts
                )),
            );
            // A limit hit mid-strike is still an auth problem; name that
            // instead of the generic reconnect ceiling.
            let reason = match pending_auth_rejection.take() {
                Some(rejection) => rejection,
                None => ManagedWsTerminationReason::ReconnectLimitReached {
                    limit: config.max_reconnect_attempts,
                },
            };
            terminate(&events_tx, &state_tx, reason);
            return;
        }

        let (attempt, base_delay) = backoff.next_attempt();
        if wait_reconnect_window(
            &mut cmd_rx,
            &mut cancel_rx,
            jittered_reconnect_delay_with_ratio(base_delay, config.reconnect_jitter_ratio),
            &events_tx,
            attempt,
            &state_tx,
            &mut shutdown_rx,
        )
        .await
        {
            terminate(&events_tx, &state_tx, ManagedWsTerminationReason::Requested);
            return;
        }
    }
}

async fn run_session(
    mut client: WsClient,
    config: &ManagedWsConfig,
    cmd_rx: &mut mpsc::Receiver<ManagedCommand>,
    cancel_rx: &mut mpsc::UnboundedReceiver<u64>,
    runtime: SessionRuntime<'_, '_>,
    state_tx: &watch::Sender<ManagedWsState>,
    connection_epoch: u64,
) -> SessionEnd {
    // Gaps reported against the previous connection are answered by this
    // one's snapshot; a lag from here on is this connection's problem.
    while runtime.gap_rx.try_recv().is_ok() {}

    let auth_result = race_shutdown(
        runtime.shutdown_rx,
        config.auth_timeout,
        auth::authenticate(
            &mut client,
            config,
            runtime.inbound,
            runtime.resume_session_id,
        ),
    )
    .await;
    match auth_result {
        Phase::ShutdownRequested => {
            return finish_unsplit(client, config, SessionEnd::CloseRequested).await;
        }
        Phase::Done(Ok(())) => {}
        Phase::Done(Err(err)) => {
            send_event(runtime.events_tx, ManagedWsEvent::Error(err.to_string()));
            if let Some(reason) = err.termination_reason() {
                return finish_unsplit(client, config, SessionEnd::Terminated(reason)).await;
            }
            return SessionEnd::AuthenticationFailed;
        }
        Phase::TimedOut => {
            send_event(
                runtime.events_tx,
                ManagedWsEvent::Error("authentication timed out".to_string()),
            );
            return SessionEnd::AuthenticationFailed;
        }
    }
    transition_state(
        state_tx,
        runtime.events_tx,
        ManagedWsState::Authenticated { connection_epoch },
        ManagedWsEvent::Authenticated,
    );
    let authenticated_at = Instant::now();

    let readiness_result = race_shutdown(
        runtime.shutdown_rx,
        config.readiness_timeout,
        ready::establish(&mut client, config, runtime.inbound, runtime.subscriptions),
    )
    .await;
    match readiness_result {
        Phase::ShutdownRequested => {
            return finish_unsplit(client, config, SessionEnd::CloseRequested).await;
        }
        Phase::Done(Ok(())) => {
            transition_state(
                state_tx,
                runtime.events_tx,
                ManagedWsState::Reconciled { connection_epoch },
                ManagedWsEvent::Reconciled,
            );
            transition_state(
                state_tx,
                runtime.events_tx,
                ManagedWsState::Ready { connection_epoch },
                ManagedWsEvent::Ready,
            );
        }
        Phase::Done(Err(err)) => {
            send_event(runtime.events_tx, ManagedWsEvent::Error(err.to_string()));
            let end = err.into_session_end().finish(authenticated_at);
            return finish_unsplit(client, config, end).await;
        }
        Phase::TimedOut => {
            send_event(
                runtime.events_tx,
                ManagedWsEvent::Error("readiness barrier timed out".to_string()),
            );
            return AuthenticatedSessionEnd::Disconnected.finish(authenticated_at);
        }
    }

    // From here the socket is split: this task reads and correlates, the
    // writer task preserves FIFO for trading commands and prioritizes transport control.
    let (sink, mut reader) = client.split();
    let (lanes, mut writer_task) = writer::spawn(
        sink,
        config.write_timeout,
        config.command_buffer,
        runtime.events_tx.clone(),
    );
    let ping = match PreparedClientMessage::new(&ClientMessage::Ping) {
        Ok(ping) => ping,
        Err(error) => {
            send_event(runtime.events_tx, ManagedWsEvent::Error(error.to_string()));
            writer_task.abort();
            return AuthenticatedSessionEnd::Disconnected.finish(authenticated_at);
        }
    };

    let mut ping_timer = interval(config.ping_interval);
    ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pong_deadline = None;
    let mut subscription_deadline = None;
    let mut subscription_written = None;
    // The deadline starts when the ping is on the wire, not when it is queued
    // behind other control frames.
    let mut ping_written: Option<oneshot::Receiver<Result<(), ManagedWsError>>> = None;

    let end = loop {
        tokio::select! {
            _ = wait_for_shutdown(runtime.shutdown_rx) => {
                break AuthenticatedSessionEnd::CloseRequested;
            }
            Some(()) = runtime.gap_rx.recv() => {
                send_event(
                    runtime.events_tx,
                    ManagedWsEvent::Error("subscriber fell behind the inbound ring; reconnecting".to_string()),
                );
                break AuthenticatedSessionEnd::Disconnected;
            }
            _ = &mut writer_task => {
                break AuthenticatedSessionEnd::Disconnected;
            }
            maybe_cmd = cmd_rx.recv() => {
                if let Some(end) = io::handle_command(maybe_cmd, &lanes, runtime.tracker, connection_epoch, runtime.subscriptions, &mut subscription_written) {
                    break end;
                }
            }
            written = async {
                match subscription_written.as_mut() {
                    Some(receipt) => receipt.await,
                    None => std::future::pending().await,
                }
            }, if subscription_written.is_some() => {
                subscription_written = None;
                match written {
                    Ok(written_at) if runtime.subscriptions.has_pending() => {
                        subscription_deadline = Some(written_at + config.readiness_timeout);
                    }
                    Ok(_) => {}
                    Err(_) => break AuthenticatedSessionEnd::Disconnected,
                }
            }
            _ = wait_for_deadline(subscription_deadline), if subscription_deadline.is_some() => {
                send_event(runtime.events_tx, ManagedWsEvent::Error("subscription acknowledgement timed out".to_string()));
                break AuthenticatedSessionEnd::Disconnected;
            }
            maybe_await_id = cancel_rx.recv() => {
                let Some(await_id) = maybe_await_id else {
                    break AuthenticatedSessionEnd::CloseRequested;
                };
                runtime.tracker.cancel(await_id);
            }
            _ = ping_timer.tick(), if pong_deadline.is_none() && ping_written.is_none() => {
                let (written_tx, written_rx) = oneshot::channel();
                match lanes.frame(ping.clone(), Some(written_tx)) {
                    Ok(()) => ping_written = Some(written_rx),
                    // A stalled writer is ended by its own write timeout.
                    Err((LaneFull::Full, _)) => {}
                    Err((LaneFull::Closed, _)) => break AuthenticatedSessionEnd::Disconnected,
                }
            }
            written = wait_written(ping_written.as_mut()), if ping_written.is_some() => {
                ping_written = None;
                if written {
                    pong_deadline = Some(Instant::now() + config.pong_timeout);
                }
            }
            _ = wait_for_deadline(pong_deadline), if pong_deadline.is_some() => {
                send_event(
                    runtime.events_tx,
                    ManagedWsEvent::Error("pong deadline exceeded".to_string()),
                );
                break AuthenticatedSessionEnd::Disconnected;
            }
            read_result = io::read_ws(&mut reader, config.ws_read_timeout) => {
                let read_result = match read_result {
                    Some(Ok(WsFrame::Ping(payload))) => {
                        if lanes.pong(payload) == Err(LaneFull::Closed) {
                            break AuthenticatedSessionEnd::Disconnected;
                        }
                        continue;
                    }
                    Some(Ok(WsFrame::Pong)) => continue,
                    Some(Ok(WsFrame::Message(message))) => Some(Ok(message)),
                    Some(Err(error)) => Some(Err(error)),
                    None => None,
                };
                if let Some(Ok(message)) = &read_result {
                    runtime.subscriptions.resolve(message);
                    if !runtime.subscriptions.has_pending() { subscription_deadline = None; }
                }
                if matches!(read_result, Some(Ok(ServerMessage::Pong(_)))) {
                    pong_deadline = None;
                }
                if let Some(end) = session_message::handle_ws_read(
                    read_result,
                    &lanes,
                    config,
                    runtime.tracker,
                    runtime.inbound,
                    runtime.events_tx,
                    runtime.shutdown_rx,
                ).await {
                    break end;
                }
            }
        }
    };

    match end {
        AuthenticatedSessionEnd::CloseRequested | AuthenticatedSessionEnd::Terminated(_) => {
            lanes.close(writer_task, config.write_timeout).await;
        }
        AuthenticatedSessionEnd::Disconnected | AuthenticatedSessionEnd::CredentialsExpired => {
            drop(lanes);
            writer_task.abort();
        }
    }
    end.finish(authenticated_at)
}

/// Resolves to whether the queued ping reached the socket. A dropped ticket
/// means the writer died, which its own task arm reports.
async fn wait_written(rx: Option<&mut oneshot::Receiver<Result<(), ManagedWsError>>>) -> bool {
    match rx {
        Some(rx) => matches!(rx.await, Ok(Ok(()))),
        None => std::future::pending().await,
    }
}

/// Close the socket for ends that are final; a reconnecting end just drops it.
async fn finish_unsplit(client: WsClient, config: &ManagedWsConfig, end: SessionEnd) -> SessionEnd {
    if matches!(end, SessionEnd::CloseRequested | SessionEnd::Terminated(_)) {
        let _ = timeout(config.write_timeout, client.close()).await;
    }
    end
}

pub(super) fn terminate(
    events_tx: &broadcast::Sender<ManagedWsEvent>,
    state_tx: &watch::Sender<ManagedWsState>,
    reason: ManagedWsTerminationReason,
) {
    transition_state(
        state_tx,
        events_tx,
        ManagedWsState::Closed {
            reason: reason.clone(),
        },
        ManagedWsEvent::Terminated(reason),
    );
}

/// Outcome of a connection phase that races a deadline against shutdown.
enum Phase<T> {
    ShutdownRequested,
    TimedOut,
    Done(T),
}

/// Run one phase of the connection lifecycle, letting shutdown pre-empt it.
///
/// Shutdown is polled first so a close request wins a tie with a phase that
/// completed in the same wakeup.
async fn race_shutdown<F: std::future::Future>(
    shutdown_rx: &mut watch::Receiver<bool>,
    deadline: Duration,
    phase: F,
) -> Phase<F::Output> {
    tokio::select! {
        biased;
        () = wait_for_shutdown(shutdown_rx) => Phase::ShutdownRequested,
        result = timeout(deadline, phase) => match result {
            Ok(output) => Phase::Done(output),
            Err(_) => Phase::TimedOut,
        },
    }
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    if *shutdown_rx.borrow() {
        return;
    }
    let _ = shutdown_rx.changed().await;
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    } else {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
