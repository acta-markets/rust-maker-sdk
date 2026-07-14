use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::oneshot;
use uuid::Uuid;

use crate::types::ids::OrderId;
use crate::ws::types::{BatchQuoteResult, ClientMessage, ServerMessage};

use super::SendAwaitError;

type AwaitResult = Result<Arc<ServerMessage>, SendAwaitError>;
type AwaitSender = oneshot::Sender<AwaitResult>;
type RegisterError = (SendAwaitError, AwaitSender);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CorrelationKey {
    Request(Uuid),
    Quote(OrderId),
    Batch(Vec<OrderId>),
    CancelQuote(Uuid),
    CancelRfq(Uuid),
    CreateRfq(Uuid),
}

impl CorrelationKey {
    fn for_client(message: &ClientMessage) -> Option<Vec<Self>> {
        match message {
            ClientMessage::Quote(message) => Some(vec![Self::Quote(message.order_id)]),
            ClientMessage::ReplaceQuote(message) => Some(vec![Self::Quote(message.order_id)]),
            ClientMessage::BatchQuotes(message) => Some(vec![Self::Batch(sorted_order_ids(
                message.quotes.iter().map(|quote| quote.order_id),
            ))]),
            ClientMessage::CancelQuote(message) => Some(vec![
                Self::Request(message.request_id),
                Self::CancelQuote(message.rfq_id),
            ]),
            ClientMessage::CancelRfq(message) => Some(vec![
                Self::Request(message.request_id),
                Self::CancelRfq(message.rfq_id),
            ]),
            ClientMessage::RfqRequest(message) => message
                .client_request_id
                .map(|request_id| vec![Self::CreateRfq(request_id)]),
            _ => message
                .request_id()
                .map(|request_id| vec![Self::Request(request_id)]),
        }
    }

    fn for_server(message: &ServerMessage) -> Option<Self> {
        match message {
            ServerMessage::QuoteAcknowledged(message) => Some(Self::Quote(message.order_id)),
            ServerMessage::QuoteRejected(message) => Some(Self::Quote(message.order_id)),
            ServerMessage::BatchQuotesAck(message) => Some(Self::Batch(sorted_order_ids(
                message.results.iter().map(|result| match result {
                    BatchQuoteResult::Acknowledged(quote) => quote.order_id,
                    BatchQuoteResult::Rejected(quote) => quote.order_id,
                }),
            ))),
            ServerMessage::QuoteCancelled(message) => Some(Self::CancelQuote(message.rfq_id)),
            ServerMessage::RfqClosed(message) => Some(Self::CancelRfq(message.rfq_id)),
            ServerMessage::RfqCreated(message) => message.client_request_id.map(Self::CreateRfq),
            _ => message.request_id().map(Self::Request),
        }
    }
}

fn sorted_order_ids(ids: impl Iterator<Item = OrderId>) -> Vec<OrderId> {
    let mut ids = ids.collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

struct PendingAwait {
    keys: Vec<CorrelationKey>,
    tx: AwaitSender,
}

pub(crate) struct AwaitTracker {
    max_pending: usize,
    by_id: HashMap<u64, PendingAwait>,
    by_key: HashMap<CorrelationKey, u64>,
}

impl AwaitTracker {
    pub(crate) fn new(max_pending: usize) -> Self {
        Self {
            max_pending,
            by_id: HashMap::with_capacity(max_pending.min(1024)),
            by_key: HashMap::with_capacity(max_pending.min(1024)),
        }
    }

    pub(crate) fn register(
        &mut self,
        await_id: u64,
        message: &ClientMessage,
        tx: AwaitSender,
    ) -> Result<(), RegisterError> {
        let Some(keys) = CorrelationKey::for_client(message) else {
            return Err((SendAwaitError::NoCorrelationKey, tx));
        };
        if self.by_id.len() >= self.max_pending {
            return Err((
                SendAwaitError::TooManyPending {
                    limit: self.max_pending,
                },
                tx,
            ));
        }
        if keys.iter().any(|key| self.by_key.contains_key(key)) {
            return Err((SendAwaitError::DuplicateInFlight, tx));
        }

        for key in &keys {
            self.by_key.insert(key.clone(), await_id);
        }
        self.by_id.insert(await_id, PendingAwait { keys, tx });
        Ok(())
    }

    pub(crate) fn cancel(&mut self, await_id: u64) -> Option<AwaitSender> {
        let pending = self.by_id.remove(&await_id)?;
        for key in &pending.keys {
            self.by_key.remove(key);
        }
        Some(pending.tx)
    }

    pub(crate) fn take_for_message(&mut self, message: &ServerMessage) -> Option<AwaitSender> {
        let key = CorrelationKey::for_server(message)?;
        let await_id = *self.by_key.get(&key)?;
        self.cancel(await_id)
    }

    pub(crate) fn drain_all(&mut self) {
        self.by_key.clear();
        for (_, pending) in self.by_id.drain() {
            let _ = pending.tx.send(Err(SendAwaitError::Disconnected));
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.by_id.len()
    }
}
