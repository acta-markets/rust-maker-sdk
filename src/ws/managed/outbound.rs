use crate::ws::client::PreparedClientMessage;
use crate::ws::types::ClientMessage;
use tokio::sync::oneshot;

use super::ManagedWsError;

/// Receipt for a message queued with `try_send`.
///
/// Dropping before dequeue cancels the send. Retain the ticket until
/// [`wait()`](Self::wait) reports the socket-write result, not a server ACK.
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
    #[error("taker_buy_unsupported")]
    TakerBuyUnsupported,
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
    if has_taker_buy(message) {
        return Err(OutboundMessageError::TakerBuyUnsupported);
    }
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

fn has_taker_buy(message: &ClientMessage) -> bool {
    match message {
        ClientMessage::RfqRequest(request) => request.side.is_taker_buy(),
        ClientMessage::Quote(quote) => quote.side.is_taker_buy(),
        ClientMessage::ReplaceQuote(quote) => quote.side.is_taker_buy(),
        ClientMessage::BatchQuotes(batch) => {
            batch.quotes.iter().any(|quote| quote.side.is_taker_buy())
        }
        ClientMessage::CheckQuote(request) => request.side.is_taker_buy(),
        _ => false,
    }
}
