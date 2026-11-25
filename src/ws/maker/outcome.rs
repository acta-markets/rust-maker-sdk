use std::time::Instant;

use uuid::Uuid;

use crate::types::{
    OptionTerms, OrderId, QuoteExpiry, QuoteExpiryReason, RfqAvailableAgainReason, RfqCloseReason,
    TakerSide,
};
use crate::ws::managed::ManagedInbound;
use crate::ws::types::{QuoteFill, QuoteRejectReason, QuoteState, ServerMessage};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum QuoteOutcome {
    Pending,
    Acked,
    Selected,
    Rejected(QuoteRejectReason),
    Filled(Box<QuoteFill>),
    Expired,
    /// The order ended; quote again with a fresh order valid at least until `min_valid_until`.
    RefreshRequested {
        terms: OptionTerms,
        side: TakerSide,
        min_valid_until: QuoteExpiry,
        reason: QuoteExpiryReason,
    },
    Cancelled,
    /// Selected, but the trade did not happen; the quote left the book.
    Released(RfqAvailableAgainReason),
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
            ServerMessage::QuoteUpdate(m)
                if m.order_id == self.order_id && m.rfq_id == self.rfq_id =>
            {
                match &m.state {
                    QuoteState::Live { .. } if matches!(self.outcome, QuoteOutcome::Selected) => {
                        return;
                    }
                    QuoteState::Live { .. } => QuoteOutcome::Acked,
                    QuoteState::RefreshRequested {
                        terms,
                        side,
                        min_valid_until,
                        reason,
                        ..
                    } => QuoteOutcome::RefreshRequested {
                        terms: *terms,
                        side: *side,
                        min_valid_until: *min_valid_until,
                        reason: *reason,
                    },
                    QuoteState::Expired { .. } => QuoteOutcome::Expired,
                    QuoteState::Selected(_) => QuoteOutcome::Selected,
                    QuoteState::Filled(fill) => QuoteOutcome::Filled(Box::new(fill.clone())),
                    QuoteState::Rejected { .. }
                        if !matches!(self.outcome, QuoteOutcome::Pending) =>
                    {
                        return;
                    }
                    QuoteState::Rejected { reason, .. } => QuoteOutcome::Rejected(reason.clone()),
                    QuoteState::Released { reason, .. } => QuoteOutcome::Released(*reason),
                    QuoteState::Lost { reason, .. } => {
                        QuoteOutcome::Lost(QuoteLossReason::RfqClosed(*reason))
                    }
                    QuoteState::Cancelled { .. } => QuoteOutcome::Cancelled,
                }
            }
            _ => return,
        };
        self.outcome = next;
    }
}

#[cfg(test)]
mod tests;
