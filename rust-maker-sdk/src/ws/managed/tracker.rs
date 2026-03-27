use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::oneshot;
use uuid::Uuid;

use crate::ws::types::ServerMessage;

use super::SendAwaitError;

pub(crate) struct PendingAwait {
    pub expected: &'static str,
    pub request_id: Option<Uuid>,
    pub tx: oneshot::Sender<Result<Arc<ServerMessage>, SendAwaitError>>,
}

pub(crate) struct AwaitTracker {
    pending: VecDeque<PendingAwait>,
}

impl AwaitTracker {
    pub(crate) fn new() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }

    pub(crate) fn register(
        &mut self,
        expected: &'static str,
        request_id: Option<Uuid>,
        tx: oneshot::Sender<Result<Arc<ServerMessage>, SendAwaitError>>,
    ) {
        self.pending.push_back(PendingAwait {
            expected,
            request_id,
            tx,
        });
    }

    pub(crate) fn take_for_message(
        &mut self,
        message: &ServerMessage,
    ) -> Option<oneshot::Sender<Result<Arc<ServerMessage>, SendAwaitError>>> {
        if self.pending.is_empty() {
            return None;
        }

        // Session-level errors have no request_id. If there is exactly one
        // pending awaiter we can deliver it unambiguously; with multiple
        // awaiters we cannot determine which request triggered the error,
        // so we log and let them time out.
        if matches!(message, ServerMessage::Error(_)) {
            if self.pending.len() == 1 {
                return self.pending.pop_front().map(|p| p.tx);
            }
            if !self.pending.is_empty() {
                tracing::warn!(
                    pending = self.pending.len(),
                    "session-level ServerMessage::Error with multiple pending awaiters; cannot route"
                );
            }
            return None;
        }

        if let ServerMessage::RequestError(env) = message {
            if let Some(idx) = self
                .pending
                .iter()
                .position(|p| p.request_id == Some(env.request_id))
            {
                return self.pending.remove(idx).map(|p| p.tx);
            }
            return None;
        }

        let variant_name: &'static str = message.into();
        let response_request_id = message.request_id();

        if let Some(request_id) = response_request_id {
            if let Some(idx) = self
                .pending
                .iter()
                .position(|p| p.expected == variant_name && p.request_id == Some(request_id))
            {
                return self.pending.remove(idx).map(|p| p.tx);
            }
            let mut candidates = self
                .pending
                .iter()
                .enumerate()
                .filter(|(_, p)| p.expected == variant_name && p.request_id.is_none());
            let (idx, _) = candidates.next()?;
            if candidates.next().is_some() {
                return None;
            }
            return self.pending.remove(idx).map(|p| p.tx);
        }

        let mut candidates = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, p)| p.expected == variant_name);
        let (idx, _) = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        self.pending.remove(idx).map(|p| p.tx)
    }

    pub(crate) fn remove_latest(
        &mut self,
    ) -> Option<oneshot::Sender<Result<Arc<ServerMessage>, SendAwaitError>>> {
        self.pending.pop_back().map(|p| p.tx)
    }

    pub(crate) fn drain_all(&mut self) {
        for req in self.pending.drain(..) {
            let _ = req.tx.send(Err(SendAwaitError::Disconnected));
        }
    }
}
