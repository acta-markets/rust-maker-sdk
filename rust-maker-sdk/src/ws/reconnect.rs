use std::time::Duration;

use rand::RngCore;

pub fn next_reconnect_delay(current: Duration, max: Duration) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > max { max } else { doubled }
}

/// Apply +/-20% jitter to avoid synchronized reconnect spikes.
pub fn jittered_reconnect_delay(base: Duration) -> Duration {
    let base_ms = base.as_millis() as u64;
    if base_ms <= 1 {
        return base;
    }

    let spread = (base_ms / 5).max(1);
    let window = spread.saturating_mul(2).saturating_add(1);
    let jitter = rand::rng().next_u64() % window;
    let jittered_ms = base_ms.saturating_sub(spread).saturating_add(jitter).max(1);
    Duration::from_millis(jittered_ms)
}
