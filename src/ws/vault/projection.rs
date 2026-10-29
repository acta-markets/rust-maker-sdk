use crate::ws::managed::{ManagedMessageReceiver, ManagedReceiveError};
use crate::ws::types::{ServerMessage, VaultProjectionAdvancedData};

/// One committed vault projection step, with its place in the inbound ring.
#[derive(Debug, Clone)]
pub struct VaultProjection {
    pub connection_epoch: u64,
    pub sequence: u64,
    pub data: VaultProjectionAdvancedData,
}

impl VaultProjection {
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.data.version
    }

    #[must_use]
    pub const fn follows(&self, previous_version: u64) -> bool {
        matches!(previous_version.checked_add(1), Some(next) if next == self.data.version)
    }
}

/// Yields only `VaultProjectionAdvanced`.
pub struct VaultProjectionReceiver {
    inner: ManagedMessageReceiver,
}

impl VaultProjectionReceiver {
    pub(super) const fn new(inner: ManagedMessageReceiver) -> Self {
        Self { inner }
    }

    pub async fn recv(&mut self) -> Result<VaultProjection, ManagedReceiveError> {
        loop {
            let inbound = self.inner.recv().await?;
            if let ServerMessage::VaultProjectionAdvanced(data) = inbound.message() {
                return Ok(VaultProjection {
                    connection_epoch: inbound.connection_epoch,
                    sequence: inbound.sequence,
                    data: data.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacency_rejects_replays_backwards_steps_and_wraparound() {
        let projection = |version| VaultProjection {
            connection_epoch: 1,
            sequence: 1,
            data: VaultProjectionAdvancedData {
                vault_pda: crate::Address32::new([0; 32]),
                version,
                slot: 1,
                transaction_index: 0,
                signature: String::new(),
                event_index: 0,
            },
        };
        assert!(projection(43).follows(42));
        assert!(!projection(42).follows(42));
        assert!(!projection(41).follows(42));
        assert!(!projection(44).follows(42));
        assert!(!projection(0).follows(u64::MAX));
    }
}
