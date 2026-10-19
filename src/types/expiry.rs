use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Signed-quote expiry in whole Unix seconds.
/// [`SystemTime`] conversion truncates to match the signed preimage.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct QuoteExpiry(u64);

impl QuoteExpiry {
    #[must_use]
    pub const fn from_unix_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Truncate a [`SystemTime`] to whole Unix seconds.
    ///
    /// Returns `None` before the Unix epoch.
    #[must_use]
    pub fn from_system_time(time: SystemTime) -> Option<Self> {
        Some(Self(time.duration_since(UNIX_EPOCH).ok()?.as_secs()))
    }

    /// Truncate `now + lifetime` to whole Unix seconds.
    ///
    /// Returns `None` if addition overflows [`SystemTime`] or the result is before the epoch.
    #[must_use]
    pub fn after(lifetime: Duration) -> Option<Self> {
        Self::from_system_time(SystemTime::now().checked_add(lifetime)?)
    }

    #[must_use]
    pub const fn as_unix_seconds(self) -> u64 {
        self.0
    }

    /// The equivalent [`SystemTime`], or `None` outside the platform's time range.
    #[must_use]
    pub fn to_system_time(self) -> Option<SystemTime> {
        UNIX_EPOCH.checked_add(Duration::from_secs(self.0))
    }

    /// Time remaining, or `None` for an expired or unrepresentable timestamp.
    #[must_use]
    pub fn remaining_from(self, now: SystemTime) -> Option<Duration> {
        self.to_system_time()?.duration_since(now).ok()
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

#[cfg(test)]
#[path = "expiry_tests.rs"]
mod tests;
