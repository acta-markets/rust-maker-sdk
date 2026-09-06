use std::time::Duration;

use rand::RngCore;

#[cfg(any(feature = "ws-client", test))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReconnectBackoff {
    initial: Duration,
    current: Duration,
    maximum: Duration,
    attempts: u64,
}

#[cfg(any(feature = "ws-client", test))]
impl ReconnectBackoff {
    pub(crate) const fn new(initial: Duration, maximum: Duration) -> Self {
        Self {
            initial,
            current: initial,
            maximum,
            attempts: 0,
        }
    }

    pub(crate) const fn limit_reached(self, maximum_attempts: u64) -> bool {
        maximum_attempts > 0 && self.attempts >= maximum_attempts
    }

    pub(crate) fn next_attempt(&mut self) -> (u64, Duration) {
        self.attempts = self.attempts.saturating_add(1);
        let delay = self.current;
        self.current = next_reconnect_delay(self.current, self.maximum);
        (self.attempts, delay)
    }

    pub(crate) const fn reset(&mut self) {
        self.current = self.initial;
        self.attempts = 0;
    }

    /// Reset after one maximum-backoff window without a disconnect.
    pub(crate) fn reset_after_stable_session(&mut self, authenticated_for: Duration) -> bool {
        if authenticated_for < self.maximum {
            return false;
        }
        self.reset();
        true
    }
}

pub fn next_reconnect_delay(current: Duration, max: Duration) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > max { max } else { doubled }
}

/// Apply +/-20% jitter to avoid synchronized reconnect spikes.
pub fn jittered_reconnect_delay(base: Duration) -> Duration {
    jittered_reconnect_delay_with_ratio(base, 0.2)
}

/// Apply symmetric jitter. The ratio is clamped to `0.0..=1.0`.
pub fn jittered_reconnect_delay_with_ratio(base: Duration, ratio: f64) -> Duration {
    let base_ms = base.as_millis() as u64;
    let ratio = ratio.clamp(0.0, 1.0);
    if base_ms <= 1 || ratio == 0.0 {
        return base;
    }

    let spread = ((base_ms as f64 * ratio).round() as u64).max(1);
    let window = spread.saturating_mul(2).saturating_add(1);
    let jitter = rand::rng().next_u64() % window;
    let jittered_ms = base_ms.saturating_sub(spread).saturating_add(jitter).max(1);
    Duration::from_millis(jittered_ms)
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
