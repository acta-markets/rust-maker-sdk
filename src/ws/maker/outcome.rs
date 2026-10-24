use std::time::Instant;

use uuid::Uuid;

use crate::types::{OrderId, QuoteCancelReason, QuoteFinalStatus, RfqCloseReason};
use crate::ws::managed::ManagedInbound;
use crate::ws::types::{QuoteFilledMessage, QuoteRejectReason, ServerMessage};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum QuoteOutcome {
    Pending,
    Acked,
    Selected,
    Rejected(QuoteRejectReason),
    Filled(Box<QuoteFilledMessage>),
    Expired,
    Cancelled,
    Lost(QuoteLossReason),
    Unknown(QuoteUnknownReason),
}

impl QuoteOutcome {
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending | Self::Acked | Self::Selected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuoteLossReason {
    RfqClosed(RfqCloseReason),
    RfqAccepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuoteUnknownReason {
    EpochChanged,
    TimedOut,
    StreamGap,
    StreamClosed,
}

/// Tracks one sent order in one epoch; feed received messages and call check at the deadline.
/// Mark stream gaps and closure unknown, then reconcile; unknown never proves failure.
#[derive(Debug, Clone)]
pub struct QuoteOutcomeTracker {
    rfq_id: Uuid,
    order_id: OrderId,
    connection_epoch: u64,
    deadline: Instant,
    outcome: QuoteOutcome,
}

impl QuoteOutcomeTracker {
    #[must_use]
    pub const fn new(
        rfq_id: Uuid,
        order_id: OrderId,
        connection_epoch: u64,
        deadline: Instant,
    ) -> Self {
        Self {
            rfq_id,
            order_id,
            connection_epoch,
            deadline,
            outcome: QuoteOutcome::Pending,
        }
    }

    #[must_use]
    pub const fn outcome(&self) -> &QuoteOutcome {
        &self.outcome
    }

    /// Check the current client epoch and caller-supplied monotonic time.
    pub fn check(&mut self, connection_epoch: u64, now: Instant) -> &QuoteOutcome {
        if connection_epoch != self.connection_epoch {
            self.mark_unknown(QuoteUnknownReason::EpochChanged);
        } else if now >= self.deadline {
            self.mark_unknown(QuoteUnknownReason::TimedOut);
        }
        &self.outcome
    }

    pub fn mark_unknown(&mut self, reason: QuoteUnknownReason) -> &QuoteOutcome {
        if !self.outcome.is_terminal() {
            self.outcome = QuoteOutcome::Unknown(reason);
        }
        &self.outcome
    }

    /// Ignores pre-epoch messages and compares the deadline with receipt time.
    pub fn observe(&mut self, inbound: &ManagedInbound) -> &QuoteOutcome {
        self.observe_message(
            inbound.connection_epoch,
            inbound.received_at,
            inbound.message(),
        )
    }

    fn observe_message(
        &mut self,
        epoch: u64,
        received_at: Instant,
        message: &ServerMessage,
    ) -> &QuoteOutcome {
        if epoch < self.connection_epoch {
            return &self.outcome;
        }
        self.check(epoch, received_at);
        if !self.outcome.is_terminal() {
            self.apply(message);
        }
        &self.outcome
    }

    fn apply(&mut self, message: &ServerMessage) {
        let next = match message {
            ServerMessage::QuoteAcknowledged(m)
                if m.order_id == self.order_id && m.rfq_id == self.rfq_id =>
            {
                if matches!(self.outcome, QuoteOutcome::Selected) {
                    return;
                }
                QuoteOutcome::Acked
            }
            ServerMessage::QuoteRejected(m)
                if m.order_id == self.order_id && m.rfq_id == self.rfq_id =>
            {
                QuoteOutcome::Rejected(m.reason.clone())
            }
            ServerMessage::QuoteSelected(m)
                if m.order_id == self.order_id && m.rfq_id == self.rfq_id =>
            {
                QuoteOutcome::Selected
            }
            ServerMessage::QuoteFilled(m)
                if m.order_id == self.order_id && m.rfq_id == self.rfq_id =>
            {
                QuoteOutcome::Filled(Box::new(m.clone()))
            }
            ServerMessage::QuoteExpired(m)
                if m.order_id == self.order_id && m.rfq_id == self.rfq_id =>
            {
                QuoteOutcome::Expired
            }
            ServerMessage::QuoteCancelled(m)
                if m.rfq_id == self.rfq_id && m.order_ids.contains(&self.order_id) =>
            {
                if m.reason == QuoteCancelReason::RfqAccepted {
                    QuoteOutcome::Lost(QuoteLossReason::RfqAccepted)
                } else {
                    QuoteOutcome::Cancelled
                }
            }
            ServerMessage::RfqAvailableAgain(m)
                if m.rfq_id == self.rfq_id && matches!(self.outcome, QuoteOutcome::Selected) =>
            {
                QuoteOutcome::Acked
            }
            ServerMessage::RfqClosed(m) if m.rfq_id == self.rfq_id => {
                match m
                    .your_quote
                    .as_ref()
                    .filter(|q| q.order_id == self.order_id)
                {
                    Some(q) => match q.status {
                        QuoteFinalStatus::Expired => QuoteOutcome::Expired,
                        QuoteFinalStatus::Cancelled => QuoteOutcome::Cancelled,
                        QuoteFinalStatus::Outbid => {
                            QuoteOutcome::Lost(QuoteLossReason::RfqClosed(m.reason))
                        }
                        QuoteFinalStatus::Filled | QuoteFinalStatus::Unknown => return,
                    },
                    None if m.reason == RfqCloseReason::Filled
                        && matches!(self.outcome, QuoteOutcome::Acked) =>
                    {
                        QuoteOutcome::Lost(QuoteLossReason::RfqClosed(m.reason))
                    }
                    None if m.reason == RfqCloseReason::Filled
                        || m.reason == RfqCloseReason::Unknown =>
                    {
                        return;
                    }
                    None => QuoteOutcome::Lost(QuoteLossReason::RfqClosed(m.reason)),
                }
            }
            _ => return,
        };
        self.outcome = next;
    }
}

#[cfg(test)]
mod tests;
