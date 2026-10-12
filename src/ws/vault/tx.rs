use std::time::{SystemTime, UNIX_EPOCH};

use crate::chain::vault::VaultIntentError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultTxError {
    #[error("{field} `{deadline}` is not a unix timestamp")]
    InvalidDeadline {
        field: &'static str,
        deadline: String,
    },
    #[error("{field} has passed: deadline {deadline}, now {now}")]
    Expired {
        field: &'static str,
        deadline: u64,
        now: u64,
    },
    #[error("system clock is before the unix epoch")]
    ClockBeforeEpoch,
    #[error(transparent)]
    Intent(#[from] VaultIntentError),
}

pub(super) fn ensure_deadline_ahead(
    field: &'static str,
    deadline: &str,
) -> Result<u64, VaultTxError> {
    ensure_deadline_ahead_at(field, deadline, unix_now()?)
}

pub(super) fn ensure_deadline_ahead_at(
    field: &'static str,
    deadline: &str,
    now: u64,
) -> Result<u64, VaultTxError> {
    let deadline_secs =
        deadline
            .trim()
            .parse::<u64>()
            .map_err(|_| VaultTxError::InvalidDeadline {
                field,
                deadline: deadline.to_owned(),
            })?;
    ensure_ahead(field, deadline_secs, now)
}

pub(super) fn ensure_system_time_ahead(
    field: &'static str,
    deadline: SystemTime,
) -> Result<u64, VaultTxError> {
    let deadline_secs = deadline
        .duration_since(UNIX_EPOCH)
        .map_err(|_| VaultTxError::ClockBeforeEpoch)?
        .as_secs();
    ensure_ahead(field, deadline_secs, unix_now()?)
}

fn ensure_ahead(field: &'static str, deadline: u64, now: u64) -> Result<u64, VaultTxError> {
    if now >= deadline {
        return Err(VaultTxError::Expired {
            field,
            deadline,
            now,
        });
    }
    Ok(deadline)
}

fn unix_now() -> Result<u64, VaultTxError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| VaultTxError::ClockBeforeEpoch)
}
