mod config;
mod desired_subscriptions;
mod endpoint;
mod inbound;
mod outbound;
mod reconnect_window;
mod session;
mod signer;
mod state;
#[cfg(feature = "test-helpers")]
mod test_peer;
mod tracker;
mod writer;

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::ws::client::PreparedClientMessage;
use crate::ws::error::WsClientError;
use crate::ws::types::{ClientMessage, ServerMessage};

pub use config::{GapPolicy, ManagedWsConfig, ManagedWsConfigError};
pub use endpoint::{
    MakerWsEndpoint, normalize_maker_data_ws_url, normalize_maker_ws_url,
    normalize_maker_ws_url_for_endpoint,
};
pub use inbound::{ManagedInbound, ManagedMessageReceiver, ManagedReceiveError};
pub use outbound::{OutboundMessageError, SendTicket};
pub use state::{ManagedWsState, ManagedWsTerminationReason, WaitUntilReadyError};
#[cfg(feature = "test-helpers")]
pub use test_peer::ManagedWsTestPeer;
use tracker::{AwaitRegistration, AwaitTracker};

fn send_event(tx: &broadcast::Sender<ManagedWsEvent>, event: ManagedWsEvent) {
    if tx.send(event).is_err() {
        tracing::trace!("no event receivers");
    }
}

fn transition_state(
    state_tx: &watch::Sender<ManagedWsState>,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
    state: ManagedWsState,
    event: ManagedWsEvent,
) {
    state::set_state(state_tx, state);
    send_event(events_tx, event);
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ManagedWsEvent {
    Connected,
    Authenticated,
    Reconciled,
    Ready,
    Reconnecting { attempt: u64, delay_ms: u64 },
    Disconnected,
    Error(String),
    Terminated(ManagedWsTerminationReason),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManagedWsError {
    #[error("managed ws connection is closed")]
    Closed,
    /// The command queue or the socket writer's lane is full: the socket is
    /// not draining as fast as commands arrive.
    #[error("managed ws send queue is full")]
    QueueFull,
    #[error("a subscription change is already awaiting acknowledgement")]
    SubscriptionPending,
    #[error("managed ws session is not ready")]
    NotReady,
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
#[non_exhaustive]
pub enum SendAwaitError {
    #[error("a subscription change is already awaiting acknowledgement")]
    SubscriptionPending,
    #[error("managed ws session is not ready")]
    NotReady,
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
    #[error("managed ws send queue is full")]
    QueueFull,
    #[error(transparent)]
    InvalidMessage(#[from] OutboundMessageError),
}

pub(crate) enum ManagedCommand {
    Send {
        message: PreparedClientMessage,
        connection_epoch: Option<u64>,
        tx: oneshot::Sender<Result<(), ManagedWsError>>,
    },
    SendAwait {
        await_id: u64,
        message: PreparedClientMessage,
        connection_epoch: Option<u64>,
        registration: AwaitRegistration,
        tx: oneshot::Sender<Result<Arc<ServerMessage>, SendAwaitError>>,
    },
}

#[derive(Clone)]
pub struct ManagedWsHandle {
    cmd_tx: mpsc::Sender<ManagedCommand>,
    cancel_tx: mpsc::UnboundedSender<u64>,
    gap_tx: Option<mpsc::UnboundedSender<()>>,
    messages_tx: broadcast::Sender<ManagedInbound>,
    events_tx: broadcast::Sender<ManagedWsEvent>,
    initial_messages_rx: Arc<StdMutex<Option<broadcast::Receiver<ManagedInbound>>>>,
    initial_events_rx: Arc<StdMutex<Option<broadcast::Receiver<ManagedWsEvent>>>>,
    state_rx: watch::Receiver<ManagedWsState>,
    shutdown: Arc<ShutdownOnLastHandle>,
    next_await_id: Arc<AtomicU64>,
    task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    max_batch_quotes: usize,
    max_outbound_message_size: usize,
    #[cfg(any(test, feature = "test-helpers"))]
    next_inbound_sequence: Arc<AtomicU64>,
    #[cfg(any(test, feature = "test-helpers"))]
    state_tx: watch::Sender<ManagedWsState>,
}

struct ShutdownOnLastHandle {
    tx: watch::Sender<bool>,
}

impl ShutdownOnLastHandle {
    fn request(&self) {
        self.tx.send_replace(true);
    }
}

impl Drop for ShutdownOnLastHandle {
    fn drop(&mut self) {
        self.request();
    }
}

impl ManagedWsHandle {
    /// Messages in connection-local wire order. The first subscriber receives the
    /// since-spawn backlog; later subscribers start at subscription time. Lag returns
    /// [`ManagedReceiveError::Gap`].
    pub fn subscribe_messages(&self) -> ManagedMessageReceiver {
        let initial = self
            .initial_messages_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        ManagedMessageReceiver {
            inner: initial.unwrap_or_else(|| self.messages_tx.subscribe()),
            gap_tx: self.gap_tx.clone(),
        }
    }

    /// Lifecycle events, including the since-spawn backlog for the first subscriber.
    pub fn subscribe_events(&self) -> broadcast::Receiver<ManagedWsEvent> {
        self.initial_events_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| self.events_tx.subscribe())
    }

    /// Subscribe to the latest lifecycle state.
    pub fn subscribe_state(&self) -> watch::Receiver<ManagedWsState> {
        self.state_rx.clone()
    }

    #[must_use]
    pub fn state(&self) -> ManagedWsState {
        self.state_rx.borrow().clone()
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(*self.state_rx.borrow(), ManagedWsState::Ready { .. })
    }

    pub fn ensure_ready(&self) -> Result<(), ManagedWsError> {
        self.ready_epoch().map(|_| ())
    }

    /// Wait for authentication and recovery to finish.
    pub async fn wait_until_ready(&self) -> Result<u64, WaitUntilReadyError> {
        let mut state_rx = self.state_rx.clone();
        loop {
            let state = state_rx.borrow().clone();
            match state {
                ManagedWsState::Ready { connection_epoch } => return Ok(connection_epoch),
                ManagedWsState::Closed { reason } => {
                    return Err(WaitUntilReadyError::Terminated(reason));
                }
                _ => {}
            }
            state_rx
                .changed()
                .await
                .map_err(|_| WaitUntilReadyError::StateChannelClosed)?;
        }
    }

    fn command_epoch(&self, message: &ClientMessage) -> Result<Option<u64>, ManagedWsError> {
        match self.state() {
            ManagedWsState::Ready { connection_epoch } => Ok(Some(connection_epoch)),
            _ if message.requires_ready() => Err(ManagedWsError::NotReady),
            _ => Ok(None),
        }
    }

    fn ready_epoch(&self) -> Result<u64, ManagedWsError> {
        match self.state() {
            ManagedWsState::Ready { connection_epoch } => Ok(connection_epoch),
            _ => Err(ManagedWsError::NotReady),
        }
    }

    pub async fn send(&self, message: ClientMessage) -> Result<(), ManagedWsError> {
        let connection_epoch = self.command_epoch(&message)?;
        self.send_with_epoch(message, connection_epoch).await
    }

    /// Send only in the connection epoch whose recovery the caller applied.
    /// A reconnect before dequeue rejects this command instead of rebinding it.
    pub async fn send_in_epoch(
        &self,
        message: ClientMessage,
        connection_epoch: u64,
    ) -> Result<(), ManagedWsError> {
        if self.ready_epoch()? != connection_epoch {
            return Err(ManagedWsError::NotReady);
        }
        self.send_with_epoch(message, Some(connection_epoch)).await
    }

    async fn send_with_epoch(
        &self,
        message: ClientMessage,
        connection_epoch: Option<u64>,
    ) -> Result<(), ManagedWsError> {
        let message = self.prepare_outbound(&message)?;
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagedCommand::Send {
                message,
                connection_epoch,
                tx,
            })
            .await
            .map_err(|_| ManagedWsError::Closed)?;
        SendTicket { rx }.wait().await
    }

    pub fn try_send(&self, message: ClientMessage) -> Result<SendTicket, ManagedWsError> {
        let connection_epoch = self.command_epoch(&message)?;
        let message = self.prepare_outbound(&message)?;
        let (tx, rx) = oneshot::channel();
        match self.cmd_tx.try_send(ManagedCommand::Send {
            message,
            connection_epoch,
            tx,
        }) {
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
        let connection_epoch = self
            .command_epoch(&message)
            .map_err(|_| SendAwaitError::NotReady)?;
        self.send_await_with_epoch(message, timeout_duration, connection_epoch)
            .await
    }

    pub(crate) async fn send_await_ready(
        &self,
        message: ClientMessage,
        timeout_duration: Duration,
    ) -> Result<Arc<ServerMessage>, SendAwaitError> {
        let connection_epoch = self.ready_epoch().map_err(|_| SendAwaitError::NotReady)?;
        self.send_await_with_epoch(message, timeout_duration, Some(connection_epoch))
            .await
    }

    async fn send_await_with_epoch(
        &self,
        message: ClientMessage,
        timeout_duration: Duration,
        connection_epoch: Option<u64>,
    ) -> Result<Arc<ServerMessage>, SendAwaitError> {
        let prepared = self.prepare_outbound(&message)?;
        let registration = AwaitTracker::prepare(&message)?;
        let await_id = self.next_await_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + timeout_duration;
        let command = ManagedCommand::SendAwait {
            await_id,
            message: prepared,
            connection_epoch,
            registration,
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

    fn prepare_outbound(
        &self,
        message: &ClientMessage,
    ) -> Result<PreparedClientMessage, OutboundMessageError> {
        outbound::prepare_outbound(
            message,
            self.max_batch_quotes,
            self.max_outbound_message_size,
        )
    }

    pub async fn close(&self) -> Result<(), ManagedWsError> {
        self.shutdown.request();
        let mut task = self.task.lock().await;
        if let Some(handle) = task.as_mut() {
            let result = handle.await;
            task.take();
            result.map_err(ManagedWsError::TaskJoin)?;
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-helpers"))]
    fn make_test_handle(
        cmd_buffer: usize,
        broadcast_buffer: usize,
    ) -> (Self, mpsc::Receiver<ManagedCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(cmd_buffer);
        let (cancel_tx, _cancel_rx) = mpsc::unbounded_channel();
        let (messages_tx, initial_messages_rx) =
            broadcast::channel::<ManagedInbound>(broadcast_buffer);
        let (events_tx, initial_events_rx) = broadcast::channel(broadcast_buffer);
        let (state_tx, state_rx) = watch::channel(ManagedWsState::Connecting);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        (
            Self {
                cmd_tx,
                cancel_tx,
                gap_tx: None,
                messages_tx,
                events_tx,
                initial_messages_rx: Arc::new(StdMutex::new(Some(initial_messages_rx))),
                initial_events_rx: Arc::new(StdMutex::new(Some(initial_events_rx))),
                state_rx,
                shutdown: Arc::new(ShutdownOnLastHandle { tx: shutdown_tx }),
                next_await_id: Arc::new(AtomicU64::new(1)),
                task: Arc::new(tokio::sync::Mutex::new(None)),
                max_batch_quotes: 50,
                max_outbound_message_size: 1024 * 1024,
                #[cfg(any(test, feature = "test-helpers"))]
                next_inbound_sequence: Arc::new(AtomicU64::new(1)),
                #[cfg(any(test, feature = "test-helpers"))]
                state_tx,
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

    #[cfg(all(feature = "test-helpers", not(test)))]
    pub fn test_handle(cmd_buffer: usize, broadcast_buffer: usize) -> (Self, ManagedWsTestPeer) {
        let (handle, commands) = Self::make_test_handle(cmd_buffer, broadcast_buffer);
        (handle, ManagedWsTestPeer::new(commands))
    }

    #[cfg(any(test, feature = "test-helpers"))]
    pub fn inject_message(&self, msg: ServerMessage) {
        let sequence = self.next_inbound_sequence.fetch_add(1, Ordering::Relaxed);
        let _ = self.messages_tx.send(ManagedInbound {
            connection_epoch: 0,
            sequence,
            received_at: std::time::Instant::now(),
            message: Arc::new(msg),
        });
    }

    #[cfg(any(test, feature = "test-helpers"))]
    pub fn inject_event(&self, event: ManagedWsEvent) {
        let _ = self.events_tx.send(event);
    }

    #[cfg(any(test, feature = "test-helpers"))]
    pub fn inject_state(&self, state: ManagedWsState) {
        self.state_tx.send_replace(state);
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
    let (gap_tx, gap_rx) = mpsc::unbounded_channel();
    let gap_tx = config.reconnect_on_gap().then_some(gap_tx);
    let (messages_tx, initial_messages_rx) = broadcast::channel::<ManagedInbound>(broadcast_buffer);
    let (events_tx, initial_events_rx) = broadcast::channel(broadcast_buffer);
    let (state_tx, state_rx) = watch::channel(ManagedWsState::Connecting);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let session = session::run_managed_ws(
        config,
        session::HandleInputs {
            cmd_rx,
            cancel_rx,
            gap_rx,
        },
        messages_tx.clone(),
        events_tx.clone(),
        state_tx.clone(),
        shutdown_rx,
    );
    // The handle keeps the broadcast senders alive, so a panicked session task
    // would leave subscribers waiting forever unless the panic is published.
    let panic_events_tx = events_tx.clone();
    let panic_state_tx = state_tx.clone();
    let task = tokio::spawn(async move {
        use futures_util::FutureExt;
        if let Err(payload) = std::panic::AssertUnwindSafe(session).catch_unwind().await {
            session::terminate(
                &panic_events_tx,
                &panic_state_tx,
                ManagedWsTerminationReason::SessionPanicked,
            );
            std::panic::resume_unwind(payload);
        }
    });

    Ok(ManagedWsHandle {
        cmd_tx,
        cancel_tx,
        gap_tx,
        messages_tx,
        events_tx,
        initial_messages_rx: Arc::new(StdMutex::new(Some(initial_messages_rx))),
        initial_events_rx: Arc::new(StdMutex::new(Some(initial_events_rx))),
        state_rx,
        shutdown: Arc::new(ShutdownOnLastHandle { tx: shutdown_tx }),
        next_await_id: Arc::new(AtomicU64::new(1)),
        task: Arc::new(tokio::sync::Mutex::new(Some(task))),
        max_batch_quotes,
        max_outbound_message_size,
        #[cfg(any(test, feature = "test-helpers"))]
        next_inbound_sequence: Arc::new(AtomicU64::new(1)),
        #[cfg(any(test, feature = "test-helpers"))]
        state_tx,
    })
}

#[cfg(test)]
#[path = "../managed_tests.rs"]
mod tests;
