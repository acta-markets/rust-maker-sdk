mod endpoint;
mod inbound;
mod outbound;
mod reconnect_window;
mod session;
mod signer;
#[cfg(feature = "test-helpers")]
mod test_peer;
mod tracker;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};

use crate::ws::client::WsTransportConfig;
use crate::ws::error::{WsClientError, WsTransportConfigError};
use crate::ws::types::{ClientMessage, HelloData, ServerMessage, StartAuthData, SubscribeData};

pub use endpoint::{
    MakerWsEndpoint, normalize_maker_data_ws_url, normalize_maker_ws_url,
    normalize_maker_ws_url_for_endpoint,
};
pub use inbound::{ManagedInbound, ManagedMessageReceiver, ManagedReceiveError};
pub use outbound::{OutboundMessageError, SendTicket};
pub use signer::ChallengeSigner;
use signer::ChallengeSigning;
#[cfg(feature = "test-helpers")]
pub use test_peer::ManagedWsTestPeer;

const MAX_INBOUND_RING_WIRE_BYTES: usize = 128 * 1024 * 1024;
const MAX_OUTBOUND_QUEUE_WIRE_BYTES: usize = 256 * 1024 * 1024;

fn send_event(tx: &broadcast::Sender<ManagedWsEvent>, event: ManagedWsEvent) {
    if tx.send(event).is_err() {
        tracing::trace!("no event receivers");
    }
}

#[derive(Clone)]
pub struct ManagedWsConfig {
    pub url: String,
    pub endpoint: MakerWsEndpoint,
    auth_pubkey: String,
    challenge_signing: ChallengeSigning,
    pub initial_subscribe: Option<SubscribeData>,
    /// Re-read authoritative state (`GetMmSummary` + `GetActiveRfqs`) on every (re)connect.
    pub auto_reconcile: bool,
    pub reconnect_delay: Duration,
    pub max_reconnect_delay: Duration,
    pub ping_interval: Duration,
    pub auth_timeout: Duration,
    pub write_timeout: Duration,
    pub command_buffer: usize,
    pub broadcast_buffer: usize,
    /// Maximum number of quotes accepted in one outbound batch.
    pub max_batch_quotes: usize,
    /// Maximum serialized size of one outbound client message.
    pub max_outbound_message_size: usize,
    pub ws_read_timeout: Option<Duration>,
    pub pong_timeout: Duration,
    pub transport: WsTransportConfig,
    pub max_pending_awaits: usize,
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
        Self::new_with_signing(
            url,
            hello,
            auth_pubkey.into(),
            ChallengeSigning::Local(challenge_signer),
        )
    }

    fn new_with_signing(
        url: impl Into<String>,
        hello: HelloData,
        auth_pubkey_str: String,
        challenge_signing: ChallengeSigning,
    ) -> Self {
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
            endpoint: MakerWsEndpoint::Quote,
            auth_pubkey: auth_pubkey_str,
            challenge_signing,
            initial_subscribe: None,
            auto_reconcile: false,
            reconnect_delay: Duration::from_millis(250),
            max_reconnect_delay: Duration::from_secs(5),
            ping_interval: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(5),
            command_buffer: 256,
            broadcast_buffer: 64,
            max_batch_quotes: 50,
            max_outbound_message_size: 1024 * 1024,
            ws_read_timeout: None,
            pong_timeout: Duration::from_secs(10),
            transport: WsTransportConfig::default(),
            max_pending_awaits: 1024,
            hello_json,
            start_auth_json,
        }
    }

    #[must_use]
    pub const fn with_endpoint(mut self, endpoint: MakerWsEndpoint) -> Self {
        self.endpoint = endpoint;
        self.auto_reconcile = matches!(endpoint, MakerWsEndpoint::Data);
        self
    }

    #[must_use]
    pub const fn with_auto_reconcile(mut self, enabled: bool) -> Self {
        self.auto_reconcile = enabled;
        self
    }

    #[must_use]
    pub fn with_initial_subscribe(mut self, subscribe: SubscribeData) -> Self {
        self.initial_subscribe = Some(subscribe);
        self
    }

    /// Validate task capacities and deadlines before spawning the session.
    ///
    /// # Errors
    /// Returns the first invalid capacity, duration, reconnect range, or transport setting.
    pub fn validate(&self) -> Result<(), ManagedWsConfigError> {
        if self.url.trim().is_empty() {
            return Err(ManagedWsConfigError::EmptyUrl);
        }
        for (field, capacity) in [
            ("command_buffer", self.command_buffer),
            ("broadcast_buffer", self.broadcast_buffer),
            ("max_pending_awaits", self.max_pending_awaits),
            ("max_batch_quotes", self.max_batch_quotes),
            ("max_outbound_message_size", self.max_outbound_message_size),
        ] {
            if capacity == 0 {
                return Err(ManagedWsConfigError::ZeroCapacity { field });
            }
        }
        for (field, duration) in [
            ("reconnect_delay", self.reconnect_delay),
            ("max_reconnect_delay", self.max_reconnect_delay),
            ("ping_interval", self.ping_interval),
            ("auth_timeout", self.auth_timeout),
            ("write_timeout", self.write_timeout),
            ("pong_timeout", self.pong_timeout),
        ] {
            if duration.is_zero() {
                return Err(ManagedWsConfigError::ZeroDuration { field });
            }
        }
        if self
            .ws_read_timeout
            .is_some_and(|duration| duration.is_zero())
        {
            return Err(ManagedWsConfigError::ZeroDuration {
                field: "ws_read_timeout",
            });
        }
        if self.reconnect_delay > self.max_reconnect_delay {
            return Err(ManagedWsConfigError::InvalidReconnectRange);
        }
        self.transport.validate()?;
        validate_memory_envelope(
            "inbound broadcast ring",
            self.broadcast_buffer,
            self.transport.max_message_size,
            MAX_INBOUND_RING_WIRE_BYTES,
        )?;
        validate_memory_envelope(
            "outbound command queue",
            self.command_buffer,
            self.max_outbound_message_size,
            MAX_OUTBOUND_QUEUE_WIRE_BYTES,
        )?;
        Ok(())
    }
}

fn validate_memory_envelope(
    queue: &'static str,
    capacity: usize,
    max_message_size: usize,
    limit: usize,
) -> Result<(), ManagedWsConfigError> {
    let configured = capacity.saturating_mul(max_message_size);
    if configured > limit {
        return Err(ManagedWsConfigError::MemoryEnvelopeTooLarge {
            queue,
            configured,
            limit,
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum ManagedWsConfigError {
    #[error("managed websocket URL must not be empty")]
    EmptyUrl,
    #[error("managed websocket capacity `{field}` must be non-zero")]
    ZeroCapacity { field: &'static str },
    #[error("managed websocket duration `{field}` must be non-zero")]
    ZeroDuration { field: &'static str },
    #[error("maximum reconnect delay must not be shorter than the initial reconnect delay")]
    InvalidReconnectRange,
    #[error("{queue} wire envelope is {configured} bytes, maximum is {limit}")]
    MemoryEnvelopeTooLarge {
        queue: &'static str,
        configured: usize,
        limit: usize,
    },
    #[error(transparent)]
    Transport(#[from] WsTransportConfigError),
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
    #[error("managed ws is disconnected")]
    Disconnected,
    #[error("websocket write timed out")]
    WriteTimeout,
    #[error("websocket write failed")]
    Write(#[source] WsClientError),
    #[error("managed websocket task failed")]
    TaskJoin(#[source] tokio::task::JoinError),
    #[error(transparent)]
    InvalidMessage(#[from] OutboundMessageError),
}

#[derive(Debug, thiserror::Error)]
pub enum SendAwaitError {
    #[error("connection closed")]
    Disconnected,
    #[error("request timed out")]
    Timeout,
    #[error("message has no stable correlation key")]
    NoCorrelationKey,
    #[error("an identical request is already awaiting a response")]
    DuplicateInFlight,
    #[error("too many pending requests (limit {limit})")]
    TooManyPending { limit: usize },
    #[error(transparent)]
    InvalidMessage(#[from] OutboundMessageError),
}

pub(crate) enum ManagedCommand {
    Send {
        message: ClientMessage,
        tx: oneshot::Sender<Result<(), ManagedWsError>>,
    },
    SendAwait {
        await_id: u64,
        message: ClientMessage,
        tx: oneshot::Sender<Result<Arc<ServerMessage>, SendAwaitError>>,
    },
    Close {
        tx: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct ManagedWsHandle {
    cmd_tx: mpsc::Sender<ManagedCommand>,
    cancel_tx: mpsc::UnboundedSender<u64>,
    messages_tx: broadcast::Sender<Arc<ManagedInbound>>,
    events_tx: broadcast::Sender<ManagedWsEvent>,
    next_await_id: Arc<AtomicU64>,
    task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    max_batch_quotes: usize,
    max_outbound_message_size: usize,
    #[cfg(any(test, feature = "test-helpers"))]
    next_inbound_sequence: Arc<AtomicU64>,
}

impl ManagedWsHandle {
    /// Inbound messages in connection-local wire order. A slow subscriber receives
    /// [`ManagedReceiveError::Gap`] instead of silently continuing after lost messages.
    pub fn subscribe_messages(&self) -> ManagedMessageReceiver {
        ManagedMessageReceiver {
            inner: self.messages_tx.subscribe(),
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ManagedWsEvent> {
        self.events_tx.subscribe()
    }

    pub async fn send(&self, message: ClientMessage) -> Result<(), ManagedWsError> {
        self.validate_outbound(&message)?;
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagedCommand::Send { message, tx })
            .await
            .map_err(|_| ManagedWsError::Closed)?;
        SendTicket { rx }.wait().await
    }

    pub fn try_send(&self, message: ClientMessage) -> Result<SendTicket, ManagedWsError> {
        self.validate_outbound(&message)?;
        let (tx, rx) = oneshot::channel();
        match self.cmd_tx.try_send(ManagedCommand::Send { message, tx }) {
            Ok(()) => Ok(SendTicket { rx }),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(ManagedWsError::Closed),
            Err(mpsc::error::TrySendError::Full(_)) => Err(ManagedWsError::QueueFull),
        }
    }

    pub async fn send_await(
        &self,
        message: ClientMessage,
        timeout_duration: Duration,
    ) -> Result<Arc<ServerMessage>, SendAwaitError> {
        self.validate_outbound(&message)?;
        let await_id = self.next_await_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + timeout_duration;
        let command = ManagedCommand::SendAwait {
            await_id,
            message,
            tx,
        };

        if tokio::time::timeout_at(deadline, self.cmd_tx.send(command))
            .await
            .map_err(|_| SendAwaitError::Timeout)?
            .is_err()
        {
            return Err(SendAwaitError::Disconnected);
        }

        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(SendAwaitError::Disconnected),
            Err(_) => {
                let _ = self.cancel_tx.send(await_id);
                Err(SendAwaitError::Timeout)
            }
        }
    }

    fn validate_outbound(&self, message: &ClientMessage) -> Result<(), OutboundMessageError> {
        outbound::validate_outbound(
            message,
            self.max_batch_quotes,
            self.max_outbound_message_size,
        )
    }

    pub async fn close(&self) -> Result<(), ManagedWsError> {
        let Some(task) = self.task.lock().await.take() else {
            return Ok(());
        };
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(ManagedCommand::Close { tx }).await.is_ok() {
            let _ = rx.await;
        }
        task.await.map_err(ManagedWsError::TaskJoin)?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-helpers"))]
    fn make_test_handle(
        cmd_buffer: usize,
        broadcast_buffer: usize,
    ) -> (Self, mpsc::Receiver<ManagedCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(cmd_buffer);
        let (cancel_tx, _cancel_rx) = mpsc::unbounded_channel();
        let (messages_tx, _) = broadcast::channel::<Arc<ManagedInbound>>(broadcast_buffer);
        let (events_tx, _) = broadcast::channel(broadcast_buffer);
        (
            Self {
                cmd_tx,
                cancel_tx,
                messages_tx,
                events_tx,
                next_await_id: Arc::new(AtomicU64::new(1)),
                task: Arc::new(tokio::sync::Mutex::new(None)),
                max_batch_quotes: 50,
                max_outbound_message_size: 1024 * 1024,
                #[cfg(any(test, feature = "test-helpers"))]
                next_inbound_sequence: Arc::new(AtomicU64::new(1)),
            },
            cmd_rx,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_handle(
        cmd_buffer: usize,
        broadcast_buffer: usize,
    ) -> (Self, mpsc::Receiver<ManagedCommand>) {
        Self::make_test_handle(cmd_buffer, broadcast_buffer)
    }

    /// Create a test handle and an opaque peer that keeps its command channel alive.
    #[cfg(all(feature = "test-helpers", not(test)))]
    pub fn test_handle(cmd_buffer: usize, broadcast_buffer: usize) -> (Self, ManagedWsTestPeer) {
        let (handle, commands) = Self::make_test_handle(cmd_buffer, broadcast_buffer);
        (handle, ManagedWsTestPeer::new(commands))
    }

    /// Inject a server message into the broadcast channel (test only).
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn inject_message(&self, msg: ServerMessage) {
        let sequence = self.next_inbound_sequence.fetch_add(1, Ordering::Relaxed);
        let _ = self.messages_tx.send(Arc::new(ManagedInbound {
            connection_epoch: 0,
            sequence,
            received_at: std::time::Instant::now(),
            message: Arc::new(msg),
        }));
    }

    /// Inject a managed ws event into the broadcast channel (test only).
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn inject_event(&self, event: ManagedWsEvent) {
        let _ = self.events_tx.send(event);
    }
}

pub fn spawn_managed_ws(config: ManagedWsConfig) -> Result<ManagedWsHandle, ManagedWsConfigError> {
    config.validate()?;
    let command_buffer = config.command_buffer;
    let broadcast_buffer = config.broadcast_buffer;
    let max_batch_quotes = config.max_batch_quotes;
    let max_outbound_message_size = config.max_outbound_message_size;
    let (cmd_tx, cmd_rx) = mpsc::channel(command_buffer);
    let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
    let (messages_tx, _) = broadcast::channel::<Arc<ManagedInbound>>(broadcast_buffer);
    let (events_tx, _) = broadcast::channel(broadcast_buffer);

    let task = tokio::spawn(session::run_managed_ws(
        config,
        cmd_rx,
        cancel_rx,
        messages_tx.clone(),
        events_tx.clone(),
    ));

    Ok(ManagedWsHandle {
        cmd_tx,
        cancel_tx,
        messages_tx,
        events_tx,
        next_await_id: Arc::new(AtomicU64::new(1)),
        task: Arc::new(tokio::sync::Mutex::new(Some(task))),
        max_batch_quotes,
        max_outbound_message_size,
        #[cfg(any(test, feature = "test-helpers"))]
        next_inbound_sequence: Arc::new(AtomicU64::new(1)),
    })
}

#[cfg(test)]
#[path = "../managed_tests.rs"]
mod tests;
