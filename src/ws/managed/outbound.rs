use crate::ws::types::ClientMessage;
use tokio::sync::oneshot;

use super::ManagedWsError;

pub struct SendTicket {
    pub(super) rx: oneshot::Receiver<Result<(), ManagedWsError>>,
}

impl SendTicket {
    pub async fn wait(self) -> Result<(), ManagedWsError> {
        self.rx.await.unwrap_or(Err(ManagedWsError::Disconnected))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OutboundMessageError {
    #[error("batch contains {actual} quotes, maximum is {limit}")]
    BatchTooLarge { actual: usize, limit: usize },
    #[error("serialized message is {actual} bytes, maximum is {limit}")]
    MessageTooLarge { actual: usize, limit: usize },
    #[error("message serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub(super) fn validate_outbound(
    message: &ClientMessage,
    max_batch_quotes: usize,
    max_message_size: usize,
) -> Result<(), OutboundMessageError> {
    if let ClientMessage::BatchQuotes(batch) = message
        && batch.quotes.len() > max_batch_quotes
    {
        return Err(OutboundMessageError::BatchTooLarge {
            actual: batch.quotes.len(),
            limit: max_batch_quotes,
        });
    }
    let serialized_size = serde_json::to_vec(message)?.len();
    if serialized_size > max_message_size {
        return Err(OutboundMessageError::MessageTooLarge {
            actual: serialized_size,
            limit: max_message_size,
        });
    }
    Ok(())
}
