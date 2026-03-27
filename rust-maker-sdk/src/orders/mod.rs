use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::wire::{WireError, decode_base58_32, decode_base58_64, encode_base58, encode_hex};

pub const ORDER_ID_LEN: usize = 32;
pub const ORDER_DOMAIN_TAG: [u8; 4] = *b"ACTA";

const TAG_LEN: usize = 4;
const CHAIN_ID_LEN: usize = 8;
const PUBKEY_LEN: usize = 32;
const BOOL_LEN: usize = 1;
const U64_LEN: usize = 8;
const HEADER_LEN: usize = TAG_LEN + CHAIN_ID_LEN + PUBKEY_LEN;

// Body field offsets (relative to HEADER_LEN).
const OFF_IS_TAKER_BUY: usize = 0;
const OFF_POSITION_TYPE: usize = OFF_IS_TAKER_BUY + BOOL_LEN;
const OFF_MARKET: usize = OFF_POSITION_TYPE + BOOL_LEN;
const OFF_STRIKE: usize = OFF_MARKET + PUBKEY_LEN;
const OFF_QUANTITY: usize = OFF_STRIKE + U64_LEN;
const OFF_GROSS_PRICE: usize = OFF_QUANTITY + U64_LEN;
const OFF_VALID_UNTIL: usize = OFF_GROSS_PRICE + U64_LEN;
const OFF_MAKER: usize = OFF_VALID_UNTIL + U64_LEN;
const OFF_TAKER: usize = OFF_MAKER + PUBKEY_LEN;
const OFF_NONCE: usize = OFF_TAKER + PUBKEY_LEN;
const _: () = assert!(HEADER_LEN + OFF_NONCE + U64_LEN == ORDER_PREIMAGE_LEN);

/// Layout: tag(4) + chain_id(8) + program_id(32) + is_taker_buy(1) +
///         position_type(1) + market(32) + strike(8) + quantity(8) +
///         gross_price(8) + valid_until(8) + maker(32) + taker(32) + nonce(8)
pub const ORDER_PREIMAGE_LEN: usize = TAG_LEN
    + CHAIN_ID_LEN
    + PUBKEY_LEN
    + BOOL_LEN
    + BOOL_LEN
    + PUBKEY_LEN
    + U64_LEN
    + U64_LEN
    + U64_LEN
    + U64_LEN
    + PUBKEY_LEN
    + PUBKEY_LEN
    + U64_LEN;

/// Trait for abstracting signing operations.
///
/// Allows using different key implementations without
/// depending on specific crates like `solana-sdk`.
pub trait SignerLike {
    fn pubkey_bytes(&self) -> [u8; 32];
    fn sign_message(&self, msg: &[u8]) -> [u8; 64];

    fn pubkey_base58(&self) -> String {
        encode_base58(&self.pubkey_bytes())
    }

    fn sign_message_base58(&self, msg: &[u8]) -> String {
        encode_base58(&self.sign_message(msg))
    }
}

/// Simple signer implementation using a cached ed25519 `SigningKey`.
///
/// The `SigningKey` is created once and reused for all signing operations,
/// avoiding per-call reconstruction overhead. `SigningKey` from `ed25519-dalek`
/// implements `Zeroize` on drop, so secret material is securely cleared.
#[derive(Clone)]
pub struct BytesSigner {
    signing_key: SigningKey,
    pubkey: [u8; 32],
}

impl Zeroize for BytesSigner {
    fn zeroize(&mut self) {
        let _ = std::mem::replace(&mut self.signing_key, SigningKey::from_bytes(&[0u8; 32]));
    }
}

impl ZeroizeOnDrop for BytesSigner {}

impl BytesSigner {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(&secret);
        let pubkey = signing_key.verifying_key().to_bytes();
        Self {
            signing_key,
            pubkey,
        }
    }

    pub fn from_keypair(keypair: &[u8; 64]) -> Self {
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&keypair[0..32]);
        Self::from_secret(secret)
    }
}

impl SignerLike for BytesSigner {
    fn pubkey_bytes(&self) -> [u8; 32] {
        self.pubkey
    }

    fn sign_message(&self, msg: &[u8]) -> [u8; 64] {
        let signature: Signature = self.signing_key.sign(msg);
        signature.to_bytes()
    }
}

#[derive(Debug, Clone)]
pub struct OrderPreimageArgs {
    pub chain_id: u64,
    pub program_id: [u8; 32],
    pub is_taker_buy: bool,
    pub position_type: u8,
    pub market: [u8; 32],
    pub strike: u64,
    pub quantity: u64,
    pub gross_price: u64,
    pub valid_until: u64,
    pub maker: [u8; 32],
    pub taker: [u8; 32],
    pub nonce: u64,
}

#[derive(Debug, Error)]
pub enum OrderError {
    #[error("invalid signing key: {0}")]
    InvalidSigningKey(String),
    #[error("invalid verifying key: {0}")]
    InvalidVerifyingKey(String),
    #[error("signature verification failed")]
    InvalidSignature,
    #[error(transparent)]
    Wire(#[from] WireError),
}

pub fn build_order_preimage(args: &OrderPreimageArgs) -> [u8; ORDER_PREIMAGE_LEN] {
    let mut buf = [0u8; ORDER_PREIMAGE_LEN];
    buf[0..TAG_LEN].copy_from_slice(&ORDER_DOMAIN_TAG);
    buf[TAG_LEN..TAG_LEN + CHAIN_ID_LEN].copy_from_slice(&args.chain_id.to_le_bytes());
    buf[TAG_LEN + CHAIN_ID_LEN..HEADER_LEN].copy_from_slice(&args.program_id);

    let b = HEADER_LEN;
    buf[b + OFF_IS_TAKER_BUY] = u8::from(args.is_taker_buy);
    buf[b + OFF_POSITION_TYPE] = args.position_type;
    buf[b + OFF_MARKET..b + OFF_MARKET + PUBKEY_LEN].copy_from_slice(&args.market);
    buf[b + OFF_STRIKE..b + OFF_STRIKE + U64_LEN].copy_from_slice(&args.strike.to_le_bytes());
    buf[b + OFF_QUANTITY..b + OFF_QUANTITY + U64_LEN].copy_from_slice(&args.quantity.to_le_bytes());
    buf[b + OFF_GROSS_PRICE..b + OFF_GROSS_PRICE + U64_LEN]
        .copy_from_slice(&args.gross_price.to_le_bytes());
    buf[b + OFF_VALID_UNTIL..b + OFF_VALID_UNTIL + U64_LEN]
        .copy_from_slice(&args.valid_until.to_le_bytes());
    buf[b + OFF_MAKER..b + OFF_MAKER + PUBKEY_LEN].copy_from_slice(&args.maker);
    buf[b + OFF_TAKER..b + OFF_TAKER + PUBKEY_LEN].copy_from_slice(&args.taker);
    buf[b + OFF_NONCE..b + OFF_NONCE + U64_LEN].copy_from_slice(&args.nonce.to_le_bytes());
    buf
}

pub fn hash_order_preimage(preimage: &[u8; ORDER_PREIMAGE_LEN]) -> [u8; ORDER_ID_LEN] {
    let digest = Sha256::digest(preimage);
    digest.into()
}

pub fn compute_order_id(args: &OrderPreimageArgs) -> [u8; ORDER_ID_LEN] {
    hash_order_preimage(&build_order_preimage(args))
}

pub fn order_id_hex(order_id: &[u8; ORDER_ID_LEN]) -> String {
    encode_hex(order_id)
}

pub fn order_preimage_hex(preimage: &[u8; ORDER_PREIMAGE_LEN]) -> String {
    encode_hex(preimage)
}

pub fn sign_order_id_bytes(
    order_id: &[u8; ORDER_ID_LEN],
    signing_key_bytes: &[u8; 32],
) -> Result<[u8; 64], OrderError> {
    let signing_key = SigningKey::from_bytes(signing_key_bytes);
    let signature: Signature = signing_key.sign(order_id);
    Ok(signature.to_bytes())
}

pub fn sign_order_id_base58(
    order_id: &[u8; ORDER_ID_LEN],
    signing_key_bytes: &[u8; 32],
) -> Result<String, OrderError> {
    let sig = sign_order_id_bytes(order_id, signing_key_bytes)?;
    Ok(encode_base58(&sig))
}

pub fn sign_order_id_with_signer<S: SignerLike>(
    order_id: &[u8; ORDER_ID_LEN],
    signer: &S,
) -> [u8; 64] {
    signer.sign_message(order_id)
}

pub fn sign_order_id_with_signer_base58<S: SignerLike>(
    order_id: &[u8; ORDER_ID_LEN],
    signer: &S,
) -> String {
    signer.sign_message_base58(order_id)
}

pub fn verify_order_id_signature_bytes(
    order_id: &[u8; ORDER_ID_LEN],
    signature: &[u8; 64],
    verifying_key_bytes: &[u8; 32],
) -> Result<(), OrderError> {
    let verifying_key = VerifyingKey::from_bytes(verifying_key_bytes)
        .map_err(|err| OrderError::InvalidVerifyingKey(err.to_string()))?;
    let signature = Signature::from_bytes(signature);
    verifying_key
        .verify(order_id, &signature)
        .map_err(|_| OrderError::InvalidSignature)
}

pub fn verify_order_id_signature_base58(
    order_id_hex: &str,
    signature_base58: &str,
    pubkey_base58: &str,
) -> Result<(), OrderError> {
    let order_id = decode_hex32(order_id_hex)?;
    let signature = decode_base58_64(signature_base58)?;
    let pubkey = decode_base58_32(pubkey_base58)?;
    verify_order_id_signature_bytes(&order_id, &signature, &pubkey)
}

pub fn sign_order_id_from_base58_keypair(
    order_id_hex: &str,
    keypair_base58: &str,
) -> Result<String, OrderError> {
    let order_id = decode_hex32(order_id_hex)?;
    let keypair = decode_base58_64(keypair_base58)?;
    let secret: [u8; 32] = keypair[0..32]
        .try_into()
        .map_err(|_| OrderError::InvalidSigningKey("expected 64-byte keypair".to_string()))?;
    sign_order_id_base58(&order_id, &secret)
}

fn decode_hex32(hex_str: &str) -> Result<[u8; 32], OrderError> {
    crate::wire::decode_hex_32(hex_str).map_err(OrderError::Wire)
}
