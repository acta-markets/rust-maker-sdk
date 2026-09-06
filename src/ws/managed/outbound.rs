use crate::ws::client::PreparedClientMessage;
use crate::ws::types::ClientMessage;
use tokio::sync::oneshot;

use super::ManagedWsError;

/// Receipt for a message queued with `try_send`.
///
/// Dropping the ticket before the session task dequeues the command cancels
/// the send: the message never reaches the wire. Fire-and-forget loses quotes
/// whenever the queue is ahead of the writer — hold every ticket until
/// [`wait()`](Self::wait) reports the socket-write result.
#[must_use = "dropping a SendTicket cancels the send if it has not been written yet"]
pub struct SendTicket {
    pub(super) rx: oneshot::Receiver<Result<(), ManagedWsError>>,
}

impl SendTicket {
    pub async fn wait(self) -> Result<(), ManagedWsError> {
        match self.rx.await {
            Ok(result) => result,
            Err(_) => Err(ManagedWsError::Disconnected),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OutboundMessageError {
    #[error("batch contains {actual} quotes, maximum is {limit}")]
    BatchTooLarge { actual: usize, limit: usize },
    #[error("serialized message is {actual} bytes, maximum is {limit}")]
    MessageTooLarge { actual: usize, limit: usize },
    #[error("message serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub(super) fn prepare_outbound(
    message: &ClientMessage,
    max_batch_quotes: usize,
    max_message_size: usize,
) -> Result<PreparedClientMessage, OutboundMessageError> {
    if let ClientMessage::BatchQuotes(batch) = message
        && batch.quotes.len() > max_batch_quotes
    {
        return Err(OutboundMessageError::BatchTooLarge {
            actual: batch.quotes.len(),
            limit: max_batch_quotes,
        });
    }
    let prepared = PreparedClientMessage::new(message)?;
    if prepared.len() > max_message_size {
        return Err(OutboundMessageError::MessageTooLarge {
            actual: prepared.len(),
            limit: max_message_size,
        });
    }
    Ok(prepared)
}
