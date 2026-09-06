use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Expiry of a signed quote, in whole Unix seconds.
///
/// A [`SystemTime`] here would admit a sub-second fraction, which the preimage
/// truncates and JSON rounds — signing one second and transmitting another.
/// Conversion always truncates, matching the preimage.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct QuoteExpiry(u64);

impl QuoteExpiry {
    /// Wrap a Unix-seconds timestamp.
    #[must_use]
    pub const fn from_unix_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Truncate a [`SystemTime`] to whole Unix seconds.
    ///
    /// Returns `None` for times before the Unix epoch or beyond `u64` seconds.
    #[must_use]
    pub fn from_system_time(time: SystemTime) -> Option<Self> {
        Some(Self(time.duration_since(UNIX_EPOCH).ok()?.as_secs()))
    }

    /// Truncate `now + lifetime` to whole Unix seconds.
    ///
    /// This is the usual way to build a quote expiry. Returns `None` if the
    /// clock is before the Unix epoch or the result overflows `u64` seconds.
    #[must_use]
    pub fn after(lifetime: Duration) -> Option<Self> {
        Self::from_system_time(SystemTime::now().checked_add(lifetime)?)
    }

    #[must_use]
    pub const fn as_unix_seconds(self) -> u64 {
        self.0
    }

    /// The equivalent [`SystemTime`], exactly on a second boundary.
    #[must_use]
    pub fn to_system_time(self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.0)
    }

    /// Seconds remaining until this expiry, or `None` once it has passed.
    #[must_use]
    pub fn remaining_from(self, now: SystemTime) -> Option<Duration> {
        self.to_system_time().duration_since(now).ok()
    }
}

impl std::fmt::Display for QuoteExpiry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u64> for QuoteExpiry {
    fn from(seconds: u64) -> Self {
        Self(seconds)
    }
}

impl From<QuoteExpiry> for u64 {
    fn from(expiry: QuoteExpiry) -> Self {
        expiry.0
    }
}

impl From<QuoteExpiry> for SystemTime {
    fn from(expiry: QuoteExpiry) -> Self {
        expiry.to_system_time()
    }
}

#[cfg(test)]
#[path = "expiry_tests.rs"]
mod tests;
