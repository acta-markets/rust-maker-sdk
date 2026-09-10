//! Unsigned Unix timestamps truncate sub-second input, matching the backend.
//! `serde_with::TimestampSeconds<i64>` rounds instead and can change a signed expiry.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{DeserializeAs, SerializeAs};

/// Whole Unix seconds, truncated. Mirrors the backend's `UnixSecondsU64`.
pub struct UnixSeconds;

impl SerializeAs<SystemTime> for UnixSeconds {
    fn serialize_as<S: Serializer>(source: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        let seconds = source
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?
            .as_secs();
        seconds.serialize(serializer)
    }
}

impl<'de> DeserializeAs<'de, SystemTime> for UnixSeconds {
    fn deserialize_as<D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        let seconds = u64::deserialize(deserializer)?;
        UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .ok_or_else(|| serde::de::Error::custom("unix timestamp exceeds SystemTime"))
    }
}

/// Whole Unix milliseconds, truncated. Mirrors the backend's `UnixMillisecondsU64`.
pub struct UnixMillis;

impl SerializeAs<SystemTime> for UnixMillis {
    fn serialize_as<S: Serializer>(source: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        let millis = u64::try_from(
            source
                .duration_since(UNIX_EPOCH)
                .map_err(serde::ser::Error::custom)?
                .as_millis(),
        )
        .map_err(serde::ser::Error::custom)?;
        millis.serialize(serializer)
    }
}

impl<'de> DeserializeAs<'de, SystemTime> for UnixMillis {
    fn deserialize_as<D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        let millis = u64::deserialize(deserializer)?;
        UNIX_EPOCH
            .checked_add(Duration::from_millis(millis))
            .ok_or_else(|| serde::de::Error::custom("unix timestamp exceeds SystemTime"))
    }
}

#[cfg(test)]
#[path = "unix_time_tests.rs"]
mod tests;
