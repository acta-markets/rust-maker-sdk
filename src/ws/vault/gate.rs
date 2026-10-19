/// The bot's admission for the trading sends an SDK driver makes.
///
/// `admit` runs `enqueue`, a non-blocking enqueue, only while the bot still
/// trades in `connection_epoch`; running it under the gate's own lock keeps a
/// concurrent close from slipping between the check and the enqueue.
pub trait SendGate {
    fn admit<R>(&self, connection_epoch: u64, enqueue: impl FnOnce() -> R) -> Option<R>;
}

/// Admits every send; the epoch check still applies.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ungated;

impl SendGate for Ungated {
    fn admit<R>(&self, _connection_epoch: u64, enqueue: impl FnOnce() -> R) -> Option<R> {
        Some(enqueue())
    }
}
