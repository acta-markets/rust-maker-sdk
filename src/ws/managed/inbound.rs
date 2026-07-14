use std::sync::Arc;

use tokio::sync::broadcast;

use crate::ws::types::ServerMessage;

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

#[derive(Debug, thiserror::Error)]
pub enum ManagedReceiveError {
    #[error("managed ws message stream closed")]
    Closed,
    #[error("managed ws subscriber lost {skipped} messages")]
    Gap { skipped: u64 },
}

pub struct ManagedMessageReceiver {
    pub(super) inner: broadcast::Receiver<Arc<ManagedInbound>>,
}

impl ManagedMessageReceiver {
    pub async fn recv(&mut self) -> Result<Arc<ManagedInbound>, ManagedReceiveError> {
        match self.inner.recv().await {
            Ok(message) => Ok(message),
            Err(broadcast::error::RecvError::Closed) => Err(ManagedReceiveError::Closed),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                Err(ManagedReceiveError::Gap { skipped })
            }
        }
    }
}
