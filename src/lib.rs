#![warn(rust_2018_idioms, unreachable_pub, unused_must_use)]
#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]
// README snippets are compiled as doc-tests (with `ws-client`) so they can't drift from the API.
#![cfg_attr(feature = "ws-client", doc = include_str!("../README.md"))]
#![cfg_attr(
    not(feature = "ws-client"),
    doc = "Rust SDK for Acta options market makers. Enable the `ws-client` feature for the WebSocket client and its documented examples."
)]

pub mod error;
pub mod nonce;
pub mod orders;
pub mod signing;
pub mod types;
pub mod wire;
pub mod ws;

pub use error::ActaSdkError;

pub const WS_PROTOCOL_VERSION: &str = "1.0.0";

// Re-export domain primitive types at crate root.
pub use types::PRICE_SCALE;
pub use types::ids::{
    Balance, ChainId, Decimals, DurationSeconds, MarketId, Nonce, OrderId, OrderVersion,
    PositionType, PositionTypeParseError, Price, Quantity, QuoteCount, RfqVersion, Slot, Strike,
    TimeoutSeconds, TradeCount, UserId, Volume,
};
pub use types::invite::{
    REFERRAL_CODE_MAX_LEN, REFERRAL_CODE_MIN_LEN, RESERVED_CODES, ReferralCode,
    ReferralCodeFormatError, TakerStatus, is_reserved,
};

#[cfg(feature = "chain")]
pub mod chain;

#[cfg(feature = "ws-client")]
pub use ws::client::{PreparedClientMessage, WsClient, WsTransportConfig};

#[cfg(feature = "ws-client")]
pub use ws::error::{WsClientError, WsResult, WsTransportConfigError};
#[cfg(all(feature = "ws-client", feature = "test-helpers"))]
pub use ws::managed::ManagedWsTestPeer;
#[cfg(feature = "ws-client")]
pub use ws::managed::{
    ChallengeSigner, MakerWsEndpoint, ManagedInbound, ManagedMessageReceiver, ManagedReceiveError,
    ManagedWsConfig, ManagedWsConfigError, ManagedWsError, ManagedWsEvent, ManagedWsHandle,
    OutboundMessageError, SendAwaitError, SendTicket, normalize_maker_data_ws_url,
    normalize_maker_ws_url, normalize_maker_ws_url_for_endpoint, spawn_managed_ws,
};
pub use ws::types::{ClientMessage, ServerMessage};

pub use nonce::{AtomicNonceGenerator, NonceError, NonceGenerator};
pub use orders::{
    BytesSigner, ORDER_DOMAIN_TAG, ORDER_ID_LEN, ORDER_PREIMAGE_LEN, OrderError, OrderPreimageArgs,
    SignerLike, build_order_preimage, compute_order_id, hash_order_preimage, order_id_hex,
    order_preimage_hex, sign_order_id_base58, sign_order_id_bytes,
    sign_order_id_from_base58_keypair, sign_order_id_with_async_signer, sign_order_id_with_signer,
    sign_order_id_with_signer_base58, verify_order_id_signature_base58,
    verify_order_id_signature_bytes,
};
pub use signing::{AsyncSignerLike, SigningError, SigningFuture};
pub use wire::*;
