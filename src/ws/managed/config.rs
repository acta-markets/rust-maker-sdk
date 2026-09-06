use std::time::Duration;

use crate::ws::client::{PreparedClientMessage, WsTransportConfig};
use crate::ws::error::WsTransportConfigError;
use crate::ws::types::{ClientMessage, FEATURE_CANCEL_ON_DISCONNECT, HelloData, SubscribeData};

use super::endpoint::MakerWsEndpoint;
use super::signer::ChallengeSigning;
use crate::orders::SignerLike;
use std::sync::Arc;

/// What the session does when a message subscriber falls behind the inbound
/// ring. The subscriber always gets `Gap`; this decides whether the session
/// also reconnects so a fresh snapshot follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum GapPolicy {
    /// Reconnect quote sessions; only surface gaps on data sessions.
    #[default]
    EndpointDefault,
    Surface,
    Reconnect,
}

// Bounds ring slots; retained payload memory depends on traffic.
const MAX_INBOUND_RING_SLOTS: usize = 65_536;
const MAX_OUTBOUND_QUEUE_WIRE_BYTES: usize = 256 * 1024 * 1024;
const SERVER_MAX_BATCH_QUOTES: usize = 50;
const SERVER_MAX_CLIENT_MESSAGE_BYTES: usize = 32 * 1024;

#[derive(Clone)]
pub struct ManagedWsConfig {
    pub url: String,
    pub endpoint: MakerWsEndpoint,
    pub(super) auth_pubkey: String,
    pub(super) challenge_signing: ChallengeSigning,
    pub(super) initial_subscribe: Option<SubscribeData>,
    pub reconnect_delay: Duration,
    pub max_reconnect_delay: Duration,
    /// Symmetric reconnect jitter ratio in the inclusive range 0.0..=1.0.
    pub reconnect_jitter_ratio: f64,
    /// Reconnects after the initial attempt. Zero means retry forever.
    pub max_reconnect_attempts: u64,
    /// Managed handshake deadline. Overrides `transport.connect_timeout` for this runtime.
    pub connect_timeout: Duration,
    pub ping_interval: Duration,
    pub auth_timeout: Duration,
    /// Deadline for initial subscription acknowledgement and recovery reads.
    /// For live subscription changes, starts after the frame is written.
    pub readiness_timeout: Duration,
    pub write_timeout: Duration,
    pub command_buffer: usize,
    pub broadcast_buffer: usize,
    /// Maximum number of quotes accepted in one outbound batch.
    pub max_batch_quotes: usize,
    /// Maximum serialized size of one outbound client message.
    pub max_outbound_message_size: usize,
    pub ws_read_timeout: Option<Duration>,
    pub pong_timeout: Duration,
    pub transport: WsTransportConfig,
    pub max_pending_awaits: usize,
    /// Ask the server to cancel this session's open quotes when it
    /// disconnects; the session terminates if the server does not enable it.
    /// Applies only to the quote endpoint.
    pub cancel_on_disconnect: bool,
    pub gap_policy: GapPolicy,
    pub(super) hello: HelloData,
}

impl ManagedWsConfig {
    /// Authenticate with a local key. The signer's own public key is the maker
    /// identity unless [`Self::with_auth_pubkey`] names a delegating owner.
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        hello: HelloData,
        signer: Arc<dyn SignerLike + Send + Sync>,
    ) -> Self {
        let auth_pubkey = signer.pubkey_base58();
        Self::new_with_signing(url, hello, auth_pubkey, ChallengeSigning::Local(signer))
    }

    pub(super) fn new_with_signing(
        url: impl Into<String>,
        hello: HelloData,
        auth_pubkey: String,
        challenge_signing: ChallengeSigning,
    ) -> Self {
        Self {
            url: url.into(),
            endpoint: MakerWsEndpoint::Quote,
            auth_pubkey,
            challenge_signing,
            initial_subscribe: None,
            reconnect_delay: Duration::from_millis(250),
            max_reconnect_delay: Duration::from_secs(5),
            reconnect_jitter_ratio: 0.2,
            max_reconnect_attempts: 0,
            connect_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(15),
            readiness_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(5),
            command_buffer: 256,
            broadcast_buffer: 64,
            max_batch_quotes: SERVER_MAX_BATCH_QUOTES,
            max_outbound_message_size: SERVER_MAX_CLIENT_MESSAGE_BYTES,
            ws_read_timeout: None,
            pong_timeout: Duration::from_secs(10),
            transport: WsTransportConfig::default(),
            max_pending_awaits: 1024,
            cancel_on_disconnect: true,
            gap_policy: GapPolicy::EndpointDefault,
            hello,
        }
    }

    /// Shorter liveness deadlines, larger queues, and reconnect on inbound gaps.
    #[must_use]
    pub const fn low_latency(mut self) -> Self {
        self.ping_interval = Duration::from_secs(2);
        self.pong_timeout = Duration::from_secs(2);
        self.write_timeout = Duration::from_secs(1);
        self.command_buffer = 1024;
        self.broadcast_buffer = 4096;
        self.gap_policy = GapPolicy::Reconnect;
        self
    }

    #[must_use]
    pub const fn with_gap_policy(mut self, policy: GapPolicy) -> Self {
        self.gap_policy = policy;
        self
    }

    #[must_use]
    pub const fn with_cancel_on_disconnect(mut self, enabled: bool) -> Self {
        self.cancel_on_disconnect = enabled;
        self
    }

    pub(super) fn reconnect_on_gap(&self) -> bool {
        match self.gap_policy {
            GapPolicy::EndpointDefault => self.endpoint == MakerWsEndpoint::Quote,
            GapPolicy::Surface => false,
            GapPolicy::Reconnect => true,
        }
    }

    /// `Hello` with every feature this configuration requires.
    pub(super) fn hello(&self) -> HelloData {
        let mut hello = self.hello.clone();
        if self.endpoint == MakerWsEndpoint::Quote
            && self.cancel_on_disconnect
            && !hello
                .features
                .iter()
                .any(|f| f == FEATURE_CANCEL_ON_DISCONNECT)
        {
            hello.features.push(FEATURE_CANCEL_ON_DISCONNECT.to_owned());
        }
        hello
    }

    /// A required feature the server's `Welcome` did not enable.
    pub(super) fn missing_required_feature(&self, enabled: &[String]) -> Option<&'static str> {
        (self.endpoint == MakerWsEndpoint::Quote
            && self.cancel_on_disconnect
            && !enabled.iter().any(|f| f == FEATURE_CANCEL_ON_DISCONNECT))
        .then_some(FEATURE_CANCEL_ON_DISCONNECT)
    }

    /// Authenticate as this maker identity instead of the signer's own key.
    ///
    /// The challenge signature is still produced by the configured signer.
    /// Required with [`Self::new_async`] when the signer holds a delegated
    /// auth key: the server expects the maker owner's public key here.
    #[must_use]
    pub fn with_auth_pubkey(mut self, pubkey: impl Into<String>) -> Self {
        self.auth_pubkey = pubkey.into();
        self
    }

    #[must_use]
    pub const fn with_endpoint(mut self, endpoint: MakerWsEndpoint) -> Self {
        self.endpoint = endpoint;
        self
    }

    #[must_use]
    pub fn with_initial_subscribe(mut self, subscribe: SubscribeData) -> Self {
        self.initial_subscribe = Some(subscribe);
        self
    }

    /// Validate task capacities and deadlines before spawning the session.
    pub fn validate(&self) -> Result<(), ManagedWsConfigError> {
        if self.url.trim().is_empty() {
            return Err(ManagedWsConfigError::EmptyUrl);
        }
        for (field, capacity) in [
            ("command_buffer", self.command_buffer),
            ("broadcast_buffer", self.broadcast_buffer),
            ("max_pending_awaits", self.max_pending_awaits),
            ("max_batch_quotes", self.max_batch_quotes),
            ("max_outbound_message_size", self.max_outbound_message_size),
        ] {
            if capacity == 0 {
                return Err(ManagedWsConfigError::ZeroCapacity { field });
            }
        }
        for (field, duration) in [
            ("reconnect_delay", self.reconnect_delay),
            ("max_reconnect_delay", self.max_reconnect_delay),
            ("connect_timeout", self.connect_timeout),
            ("ping_interval", self.ping_interval),
            ("auth_timeout", self.auth_timeout),
            ("readiness_timeout", self.readiness_timeout),
            ("write_timeout", self.write_timeout),
            ("pong_timeout", self.pong_timeout),
        ] {
            if duration.is_zero() {
                return Err(ManagedWsConfigError::ZeroDuration { field });
            }
        }
        if self
            .ws_read_timeout
            .is_some_and(|duration| duration.is_zero())
        {
            return Err(ManagedWsConfigError::ZeroDuration {
                field: "ws_read_timeout",
            });
        }
        if self.reconnect_delay > self.max_reconnect_delay {
            return Err(ManagedWsConfigError::InvalidReconnectRange);
        }
        if !(0.0..=1.0).contains(&self.reconnect_jitter_ratio) {
            return Err(ManagedWsConfigError::InvalidReconnectJitter);
        }
        validate_server_limit(
            "max_batch_quotes",
            self.max_batch_quotes,
            SERVER_MAX_BATCH_QUOTES,
        )?;
        validate_server_limit(
            "max_outbound_message_size",
            self.max_outbound_message_size,
            SERVER_MAX_CLIENT_MESSAGE_BYTES,
        )?;
        if self.endpoint == MakerWsEndpoint::Data && self.initial_subscribe.is_some() {
            return Err(ManagedWsConfigError::InitialSubscribeOnDataEndpoint);
        }
        if let Some(subscribe) = &self.initial_subscribe {
            let message = ClientMessage::Subscribe(subscribe.clone());
            let prepared = PreparedClientMessage::new(&message)
                .map_err(|_| ManagedWsConfigError::InitialSubscribeNotSerializable)?;
            if prepared.len() > self.max_outbound_message_size {
                return Err(ManagedWsConfigError::InitialSubscribeTooLarge {
                    actual: prepared.len(),
                    limit: self.max_outbound_message_size,
                });
            }
        }
        self.effective_transport().validate()?;
        validate_slot_cap(
            "broadcast_buffer",
            self.broadcast_buffer,
            MAX_INBOUND_RING_SLOTS,
        )?;
        validate_memory_envelope(
            "outbound command queue",
            self.command_buffer,
            self.max_outbound_message_size,
            MAX_OUTBOUND_QUEUE_WIRE_BYTES,
        )
    }

    pub(super) const fn effective_transport(&self) -> WsTransportConfig {
        let mut transport = self.transport;
        transport.connect_timeout = self.connect_timeout;
        transport
    }
}

fn validate_server_limit(
    field: &'static str,
    configured: usize,
    limit: usize,
) -> Result<(), ManagedWsConfigError> {
    if configured > limit {
        return Err(ManagedWsConfigError::ProtocolLimitExceeded {
            field,
            configured,
            limit,
        });
    }
    Ok(())
}

fn validate_slot_cap(
    field: &'static str,
    capacity: usize,
    limit: usize,
) -> Result<(), ManagedWsConfigError> {
    if capacity > limit {
        return Err(ManagedWsConfigError::CapacityTooLarge {
            field,
            configured: capacity,
            limit,
        });
    }
    Ok(())
}

/// Every outbound message is rejected above `max_outbound_message_size` before
/// it is queued, so unlike the inbound ring this product is a true bound on the
/// bytes the queue can hold.
fn validate_memory_envelope(
    queue: &'static str,
    capacity: usize,
    max_message_size: usize,
    limit: usize,
) -> Result<(), ManagedWsConfigError> {
    let configured = capacity.saturating_mul(max_message_size);
    if configured > limit {
        return Err(ManagedWsConfigError::MemoryEnvelopeTooLarge {
            queue,
            configured,
            limit,
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManagedWsConfigError {
    #[error("managed websocket URL must not be empty")]
    EmptyUrl,
    #[error("managed websocket capacity `{field}` must be non-zero")]
    ZeroCapacity { field: &'static str },
    #[error("managed websocket duration `{field}` must be non-zero")]
    ZeroDuration { field: &'static str },
    #[error("maximum reconnect delay must not be shorter than the initial reconnect delay")]
    InvalidReconnectRange,
    #[error("reconnect jitter ratio must be in the inclusive range 0.0..=1.0")]
    InvalidReconnectJitter,
    #[error("initial subscriptions are not supported by the maker data endpoint")]
    InitialSubscribeOnDataEndpoint,
    #[error("initial subscription is {actual} bytes, maximum is {limit}")]
    InitialSubscribeTooLarge { actual: usize, limit: usize },
    #[error("{field} is {configured}, server maximum is {limit}")]
    ProtocolLimitExceeded {
        field: &'static str,
        configured: usize,
        limit: usize,
    },
    #[error("initial subscribe cannot be serialized")]
    InitialSubscribeNotSerializable,
    #[error("{field} is {configured}, maximum is {limit}")]
    CapacityTooLarge {
        field: &'static str,
        configured: usize,
        limit: usize,
    },
    #[error("{queue} wire envelope is {configured} bytes, maximum is {limit}")]
    MemoryEnvelopeTooLarge {
        queue: &'static str,
        configured: usize,
        limit: usize,
    },
    #[error(transparent)]
    Transport(#[from] WsTransportConfigError),
}
