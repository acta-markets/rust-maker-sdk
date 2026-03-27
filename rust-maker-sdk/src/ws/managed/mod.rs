mod session;
mod tracker;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};
use uuid::Uuid;

use crate::ws::types::{ClientMessage, HelloData, ServerMessage, StartAuthData, SubscribeData};

fn send_event(tx: &broadcast::Sender<ManagedWsEvent>, event: ManagedWsEvent) {
    if tx.send(event).is_err() {
        tracing::trace!("no event receivers");
    }
}

/// Normalize a base URL to the maker WebSocket endpoint.
#[must_use]
pub fn normalize_maker_ws_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');

    // Normalize scheme.
    let url: std::borrow::Cow<'_, str> = if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}").into()
    } else if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}").into()
    } else {
        url.into()
    };

    // Append /maker if not already present.
    if url.ends_with("/maker") {
        url.into_owned()
    } else {
        format!("{url}/maker")
    }
}

type ChallengeSigner = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

#[derive(Clone)]
pub struct ManagedWsConfig {
    pub url: String,
    pub hello: HelloData,
    pub auth_pubkey: String,
    pub challenge_signer: ChallengeSigner,
    pub initial_subscribe: Option<SubscribeData>,
    pub resync_messages: Vec<ClientMessage>,
    pub reconnect_delay: Duration,
    pub max_reconnect_delay: Duration,
    pub ping_interval: Duration,
    pub auth_timeout: Duration,
    pub command_buffer: usize,
    pub broadcast_buffer: usize,
    pub ws_read_timeout: Option<Duration>,
    pub max_queued_outbound: usize,
    /// Pre-serialized Hello message JSON (Arc avoids allocation on reconnect).
    hello_json: Arc<str>,
    /// Pre-serialized StartAuth message JSON (Arc avoids allocation on reconnect).
    start_auth_json: Arc<str>,
}

impl ManagedWsConfig {
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        hello: HelloData,
        auth_pubkey: impl Into<String>,
        challenge_signer: ChallengeSigner,
    ) -> Self {
        let auth_pubkey_str: String = auth_pubkey.into();
        let hello_json: Arc<str> = serde_json::to_string(&ClientMessage::Hello(hello.clone()))
            .expect("hello serialization")
            .into();
        let start_auth_json: Arc<str> =
            serde_json::to_string(&ClientMessage::StartAuth(StartAuthData {
                pubkey: auth_pubkey_str.clone(),
            }))
            .expect("start_auth serialization")
            .into();
        Self {
            url: url.into(),
            hello,
            auth_pubkey: auth_pubkey_str,
            challenge_signer,
            initial_subscribe: None,
            resync_messages: Vec::new(),
            reconnect_delay: Duration::from_millis(250),
            max_reconnect_delay: Duration::from_secs(5),
            ping_interval: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(15),
            command_buffer: 1024,
            broadcast_buffer: 1024,
            ws_read_timeout: None,
            max_queued_outbound: 4096,
            hello_json,
            start_auth_json,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ManagedWsEvent {
    Connected,
    Authenticated,
    Reconnecting { attempt: u64, delay_ms: u64 },
    Disconnected,
    Error(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ManagedWsError {
    #[error("managed ws connection is closed")]
    Closed,
    #[error("managed ws send queue is full")]
    QueueFull,
}

#[derive(Debug, thiserror::Error)]
pub enum SendAwaitError {
    #[error("no expected response type for this message")]
    NoExpectedResponse,
    #[error("connection closed")]
    Disconnected,
    #[error("request timed out")]
    Timeout,
}

pub enum ManagedCommand {
    Send(ClientMessage),
    SendRaw(String),
    SendAwait {
        message: ClientMessage,
        expected: &'static str,
        request_id: Option<Uuid>,
        tx: oneshot::Sender<Result<Arc<ServerMessage>, SendAwaitError>>,
    },
    Close,
}

#[derive(Clone)]
pub struct ManagedWsHandle {
    cmd_tx: mpsc::Sender<ManagedCommand>,
    messages_tx: broadcast::Sender<Arc<ServerMessage>>,
    events_tx: broadcast::Sender<ManagedWsEvent>,
}

impl ManagedWsHandle {
    pub fn subscribe_messages(&self) -> broadcast::Receiver<Arc<ServerMessage>> {
        self.messages_tx.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ManagedWsEvent> {
        self.events_tx.subscribe()
    }

    pub async fn send(&self, message: ClientMessage) -> Result<(), ManagedWsError> {
        self.cmd_tx
            .send(ManagedCommand::Send(message))
            .await
            .map_err(|_| ManagedWsError::Closed)
    }

    pub fn try_send(&self, message: ClientMessage) -> Result<(), ManagedWsError> {
        match self.cmd_tx.try_send(ManagedCommand::Send(message)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(ManagedWsError::Closed),
            Err(mpsc::error::TrySendError::Full(_)) => Err(ManagedWsError::QueueFull),
        }
    }

    pub async fn send_raw(&self, json: String) -> Result<(), ManagedWsError> {
        self.cmd_tx
            .send(ManagedCommand::SendRaw(json))
            .await
            .map_err(|_| ManagedWsError::Closed)
    }

    pub fn try_send_raw(&self, json: String) -> Result<(), ManagedWsError> {
        match self.cmd_tx.try_send(ManagedCommand::SendRaw(json)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(ManagedWsError::Closed),
            Err(mpsc::error::TrySendError::Full(_)) => Err(ManagedWsError::QueueFull),
        }
    }

    pub async fn send_await(
        &self,
        message: ClientMessage,
        timeout_duration: Duration,
    ) -> Result<Arc<ServerMessage>, SendAwaitError> {
        let expected = message
            .expected_response_type()
            .ok_or(SendAwaitError::NoExpectedResponse)?;
        let request_id = message.request_id();
        let (tx, rx) = oneshot::channel();

        self.cmd_tx
            .send(ManagedCommand::SendAwait {
                message,
                expected,
                request_id,
                tx,
            })
            .await
            .map_err(|_| SendAwaitError::Disconnected)?;

        match tokio::time::timeout(timeout_duration, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(SendAwaitError::Disconnected),
            Err(_) => Err(SendAwaitError::Timeout),
        }
    }

    pub async fn close(&self) -> Result<(), ManagedWsError> {
        self.cmd_tx
            .send(ManagedCommand::Close)
            .await
            .map_err(|_| ManagedWsError::Closed)
    }

    /// Create a test handle with exposed broadcast senders for injecting messages/events.
    ///
    /// Returns `(handle, cmd_rx)` — the `cmd_rx` receives commands sent via `send()`/`try_send()`.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn test_handle(
        cmd_buffer: usize,
        broadcast_buffer: usize,
    ) -> (Self, mpsc::Receiver<ManagedCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(cmd_buffer);
        let (messages_tx, _) = broadcast::channel::<Arc<ServerMessage>>(broadcast_buffer);
        let (events_tx, _) = broadcast::channel(broadcast_buffer);
        (
            Self {
                cmd_tx,
                messages_tx,
                events_tx,
            },
            cmd_rx,
        )
    }

    /// Inject a server message into the broadcast channel (test only).
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn inject_message(&self, msg: ServerMessage) {
        let _ = self.messages_tx.send(Arc::new(msg));
    }

    /// Inject a managed ws event into the broadcast channel (test only).
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn inject_event(&self, event: ManagedWsEvent) {
        let _ = self.events_tx.send(event);
    }
}

#[must_use]
pub fn spawn_managed_ws(config: ManagedWsConfig) -> ManagedWsHandle {
    let command_buffer = config.command_buffer;
    let broadcast_buffer = config.broadcast_buffer;
    let (cmd_tx, cmd_rx) = mpsc::channel(command_buffer);
    let (messages_tx, _) = broadcast::channel::<Arc<ServerMessage>>(broadcast_buffer);
    let (events_tx, _) = broadcast::channel(broadcast_buffer);

    tokio::spawn(session::run_managed_ws(
        config,
        cmd_rx,
        messages_tx.clone(),
        events_tx.clone(),
    ));

    ManagedWsHandle {
        cmd_tx,
        messages_tx,
        events_tx,
    }
}

#[cfg(test)]
#[path = "../managed_tests.rs"]
mod tests;
