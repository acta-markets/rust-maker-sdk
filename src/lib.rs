#![warn(rust_2018_idioms, unreachable_pub, unused_must_use)]
#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod error;
pub mod nonce;
pub mod orders;
pub mod types;
pub mod wire;
pub mod ws;

pub use error::ActaSdkError;

pub const WS_PROTOCOL_VERSION: &str = "1.0.0";

// Re-export domain primitive types at crate root.
pub use types::ids::{
    Balance, ChainId, Decimals, DurationSeconds, MarketId, Nonce, OrderId, OrderVersion,
    PositionType, Price, Quantity, QuoteCount, RfqVersion, Slot, Strike, TimeoutSeconds,
    TradeCount, Volume,
};

#[cfg(feature = "chain")]
pub mod chain;

#[cfg(feature = "ws-client")]
pub use ws::client::WsClient;

#[cfg(feature = "ws-client")]
pub use ws::error::{WsClientError, WsResult};
#[cfg(feature = "ws-client")]
pub use ws::managed::*;
pub use ws::types::*;

pub use nonce::{AtomicNonceGenerator, NonceError, NonceGenerator};
pub use orders::{
    BytesSigner, ORDER_DOMAIN_TAG, ORDER_ID_LEN, ORDER_PREIMAGE_LEN, OrderError, OrderPreimageArgs,
    SignerLike, build_order_preimage, compute_order_id, hash_order_preimage, order_id_hex,
    order_preimage_hex, sign_order_id_base58, sign_order_id_bytes,
    sign_order_id_from_base58_keypair, sign_order_id_with_signer, sign_order_id_with_signer_base58,
    verify_order_id_signature_base58, verify_order_id_signature_bytes,
};
pub use wire::*;
