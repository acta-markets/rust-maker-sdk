use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};

use crate::ws::types::{ReferenceChangedData, ServerMessage};

#[derive(Debug, Clone)]
pub struct ManagedInbound {
    pub connection_epoch: u64,
    pub sequence: u64,
    pub received_at: std::time::Instant,
    pub(super) message: Arc<ServerMessage>,
}

impl ManagedInbound {
    #[must_use]
    pub fn message(&self) -> &ServerMessage {
        &self.message
    }

    #[must_use]
    pub fn message_arc(&self) -> Arc<ServerMessage> {
        Arc::clone(&self.message)
    }
}

impl AsRef<ServerMessage> for ManagedInbound {
    fn as_ref(&self) -> &ServerMessage {
        &self.message
    }
}

/// What the session broadcasts: a decoded frame, or the place of one it could not decode.
#[derive(Debug, Clone)]
pub(crate) enum InboundItem {
    Message(ManagedInbound),
    Unreadable,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManagedReceiveError {
    #[error("managed ws message stream closed")]
    Closed,
    #[error("managed ws subscriber lost {skipped} messages")]
    Gap { skipped: u64 },
}

pub struct ManagedMessageReceiver {
    pub(super) inner: broadcast::Receiver<InboundItem>,
    /// Set under `GapPolicy::Reconnect`: a lag asks the session to reconnect.
    pub(super) gap_tx: Option<mpsc::UnboundedSender<()>>,
}

impl ManagedMessageReceiver {
    pub async fn recv(&mut self) -> Result<ManagedInbound, ManagedReceiveError> {
        match self.inner.recv().await {
            Ok(InboundItem::Message(message)) => Ok(message),
            // Rebuild state as after any gap; a reconnect would only meet the same frame.
            Ok(InboundItem::Unreadable) => Err(ManagedReceiveError::Gap { skipped: 1 }),
            Err(broadcast::error::RecvError::Closed) => Err(ManagedReceiveError::Closed),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                if let Some(gap_tx) = &self.gap_tx {
                    let _ = gap_tx.send(());
                }
                Err(ManagedReceiveError::Gap { skipped })
            }
        }
    }
}

pub struct ReferenceChangeReceiver {
    inner: ManagedMessageReceiver,
}

impl ReferenceChangeReceiver {
    pub(crate) const fn new(inner: ManagedMessageReceiver) -> Self {
        Self { inner }
    }

    pub async fn recv(&mut self) -> Result<ReferenceChangedData, ManagedReceiveError> {
        loop {
            let inbound = self.inner.recv().await?;
            if let ServerMessage::ReferenceChanged(data) = inbound.message() {
                return Ok(data.clone());
            }
        }
    }
}
