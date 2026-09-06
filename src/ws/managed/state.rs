use tokio::sync::watch;

/// Latest lifecycle state of a managed connection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManagedWsState {
    Connecting,
    Connected { connection_epoch: u64 },
    Authenticated { connection_epoch: u64 },
    Reconciled { connection_epoch: u64 },
    Ready { connection_epoch: u64 },
    Reconnecting { attempt: u64, delay_ms: u64 },
    Closed { reason: ManagedWsTerminationReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManagedWsTerminationReason {
    Requested,
    ReconnectLimitReached {
        limit: u64,
    },
    SessionReplaced,
    ProtocolVersionMismatch {
        requested_version: String,
        server_version: String,
        min_supported_version: String,
    },
    AuthenticationRejected {
        reason: String,
        message: Option<String>,
    },
    /// The session task panicked. The connection is gone and will not
    /// reconnect; respawn the client to continue.
    SessionPanicked,
    /// The server did not enable a feature this configuration requires, so
    /// trading would run without a guarantee the maker asked for.
    FeatureUnsupported {
        feature: String,
    },
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WaitUntilReadyError {
    #[error("managed websocket terminated: {0:?}")]
    Terminated(ManagedWsTerminationReason),
    #[error("managed websocket state channel closed")]
    StateChannelClosed,
}

pub(super) fn set_state(tx: &watch::Sender<ManagedWsState>, state: ManagedWsState) {
    tx.send_replace(state);
}
