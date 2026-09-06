use super::*;

use crate::orders::{BytesSigner, verify_order_id_signature_bytes};
use crate::signing::{AsyncSignerLike, SigningFuture};
use crate::types::ids::ChainId;
use crate::ws::types::{MarketDescriptor, RfqOrderOption};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROGRAM: [u8; 32] = [7u8; 32];
const MARKET: [u8; 32] = [9u8; 32];
const TAKER: [u8; 32] = [11u8; 32];

fn signer() -> BytesSigner {
    BytesSigner::from_secret([3u8; 32])
}

fn broadcast(strike: u64, alternatives: &[u64]) -> RfqBroadcastMessage {
    RfqBroadcastMessage {
        rfq_id: Uuid::from_u128(42),
        market: MarketDescriptor {
            chain_id: ChainId::new(0),
            program_id: encode_base58(&PROGRAM),
            market_pda: encode_base58(&MARKET),
            underlying_mint: encode_base58(&[1u8; 32]),
            quote_mint: encode_base58(&[2u8; 32]),
            expiry_ts: UNIX_EPOCH + Duration::from_secs(2_000_000_000),
            is_put: false,
            collateral_mint: encode_base58(&[1u8; 32]),
            settlement_mint: encode_base58(&[2u8; 32]),
        },
        position_type: PositionType::CoveredCall,
        strike: Strike::new(strike),
        quantity: Quantity::new(5),
        expires_at: UNIX_EPOCH + Duration::from_secs(1_800_000_300),
        taker: encode_base58(&TAKER),
        sent_at_unix_ms: std::time::UNIX_EPOCH,
        order_options: alternatives
            .iter()
            .map(|strike| RfqOrderOption {
                strike: Strike::new(*strike),
            })
            .collect(),
    }
}

fn binding(strike: u64, alternatives: &[u64]) -> RfqBinding {
    RfqBinding::from_broadcast(&broadcast(strike, alternatives)).expect("valid identities")
}

/// The bug this module exists to prevent: a `valid_until` with a sub-second
/// fraction used to be truncated into the preimage but rounded onto the wire,
/// so roughly every other quote was signed for one second and transmitted for
/// another. The server reconstructs the preimage from the wire value, so the
/// two must be the same integer.
#[test]
fn wire_valid_until_is_the_value_that_was_signed() {
    let fractional = SystemTime::UNIX_EPOCH + Duration::from_millis(1_800_000_000_700);
    let expiry = QuoteExpiry::from_system_time(fractional).expect("in range");
    let signer = signer();
    let builder_binding = binding(100, &[]);
    let builder = builder_binding
        .quote()
        .price(Price::new(1_000))
        .valid_until(expiry)
        .nonce(Nonce::new(77));

    let args = builder
        .preimage_args(signer.pubkey_bytes())
        .expect("resolves");
    let quote = builder.sign(&signer).expect("signs");

    let json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&quote).expect("serializes")).expect("json");
    let on_the_wire = json["valid_until"].as_u64().expect("integer seconds");

    assert_eq!(args.valid_until, on_the_wire);
    assert_eq!(on_the_wire, 1_800_000_000);
}

/// The server recomputes `order_id` from the wire fields. Rebuilding the
/// preimage from the transmitted message must reproduce the same id.
#[test]
fn order_id_recomputes_from_the_transmitted_message() {
    let signer = signer();
    let bind = binding(100, &[]);
    let quote = bind
        .quote()
        .price(Price::new(2_500))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_123))
        .nonce(Nonce::new(9))
        .sign(&signer)
        .expect("signs");

    let decoded: QuoteMessage =
        serde_json::from_str(&serde_json::to_string(&quote).expect("serializes")).expect("decodes");

    let server_view = OrderPreimageArgs {
        chain_id: 0,
        program_id: PROGRAM,
        is_taker_buy: false,
        position_type: PositionType::CoveredCall,
        market: MARKET,
        strike: decoded.strike.value(),
        quantity: 5,
        gross_price: decoded.price.value(),
        valid_until: decoded.valid_until.as_unix_seconds(),
        maker: signer.pubkey_bytes(),
        taker: TAKER,
        nonce: decoded.nonce.value(),
    };

    assert_eq!(compute_order_id(&server_view), *decoded.order_id.as_bytes());
}

#[test]
fn signature_verifies_against_the_order_id() {
    let signer = signer();
    let bind = binding(100, &[]);
    let quote = bind
        .quote()
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(1))
        .sign(&signer)
        .expect("signs");

    let signature = crate::wire::decode_base58_64(&quote.signature).expect("base58 signature");
    verify_order_id_signature_bytes(
        quote.order_id.as_bytes(),
        &signature,
        &signer.pubkey_bytes(),
    )
    .expect("signature matches");
}

#[test]
fn alternative_strikes_from_order_options_are_accepted() {
    let bind = binding(100, &[110, 120]);
    assert_eq!(
        bind.offered_strikes(),
        &[Strike::new(100), Strike::new(110), Strike::new(120)]
    );

    let quote = bind
        .quote()
        .strike(Strike::new(120))
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect("signs");

    assert_eq!(quote.strike, Strike::new(120));
}

#[test]
fn a_strike_the_rfq_did_not_offer_is_refused_locally() {
    let bind = binding(100, &[110]);
    let error = bind
        .quote()
        .strike(Strike::new(999))
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("unlisted strike");

    assert!(matches!(
        error,
        QuoteBuildError::StrikeNotOffered { requested, .. } if requested == Strike::new(999)
    ));
}

#[test]
fn duplicate_order_option_does_not_repeat_the_requested_strike() {
    let bind = binding(100, &[100, 110]);
    assert_eq!(
        bind.offered_strikes(),
        &[Strike::new(100), Strike::new(110)]
    );
}

#[test]
fn missing_fields_are_named() {
    let bind = binding(100, &[]);
    let missing_price = bind
        .quote()
        .valid_until(QuoteExpiry::from_unix_seconds(1))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("no price");
    assert!(matches!(missing_price, QuoteBuildError::Missing("price")));

    let missing_expiry = bind
        .quote()
        .price(Price::new(1))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("no expiry");
    assert!(matches!(
        missing_expiry,
        QuoteBuildError::Missing("valid_until")
    ));

    let missing_nonce = bind
        .quote()
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1))
        .sign(&signer())
        .expect_err("no nonce");
    assert!(matches!(missing_nonce, QuoteBuildError::Missing("nonce")));
}

#[test]
fn a_malformed_identity_names_its_field() {
    let mut rfq = broadcast(100, &[]);
    rfq.taker = "not-base58-!!".to_string();
    let error = RfqBinding::from_broadcast(&rfq).expect_err("bad taker");
    assert!(matches!(
        error,
        QuoteBuildError::Identity { field: "taker", .. }
    ));
}

#[test]
fn a_replacement_carries_the_same_signed_identity() {
    let signer = signer();
    let bind = binding(100, &[]);
    let builder = bind
        .quote()
        .price(Price::new(3))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(4));

    let quote = builder.sign(&signer).expect("signs");
    let old = OrderId::new([1u8; 32]);
    let replacement = builder.sign(&signer).expect("signs").into_replacement(old);

    assert_eq!(replacement.old_order_id, old);
    assert_eq!(replacement.order_id, quote.order_id);
    assert_eq!(replacement.valid_until, quote.valid_until);
    assert_eq!(replacement.signature, quote.signature);
}

/// Stands in for an HSM or a remote signing service.
struct RemoteSigner(BytesSigner);

impl AsyncSignerLike for RemoteSigner {
    fn pubkey_bytes(&self) -> [u8; 32] {
        self.0.pubkey_bytes()
    }

    fn sign_message<'a>(&'a self, message: &'a [u8]) -> SigningFuture<'a> {
        Box::pin(async move { Ok(self.0.sign_message(message)) })
    }
}

/// A maker whose key lives behind a remote signer must be able to build both a
/// quote and a replacement. Replacement used to be a second signing method that
/// only accepted a local key, which left remote signers unable to replace.
#[tokio::test]
async fn a_remote_signer_can_produce_a_quote_and_a_replacement() {
    let remote = RemoteSigner(signer());
    let bind = binding(100, &[]);
    let builder = bind
        .quote()
        .price(Price::new(11))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(5));

    let quote = builder
        .sign_with_async_signer(&remote)
        .await
        .expect("remote signs");

    // Identical to what the local key produces for the same inputs.
    assert_eq!(
        quote.order_id,
        builder.sign(&signer()).expect("signs").order_id
    );

    let old = OrderId::new([2u8; 32]);
    let replacement = quote.clone().into_replacement(old);
    assert_eq!(replacement.old_order_id, old);
    assert_eq!(replacement.order_id, quote.order_id);
    assert_eq!(replacement.signature, quote.signature);
    assert_eq!(replacement.valid_until, quote.valid_until);
}

/// A delegated quote-signing key signs, but the server reconstructs the
/// preimage with the authenticated maker owner. The owner set on the builder
/// must land in the preimage while the signature stays the signer's.
#[test]
fn delegated_signer_puts_the_maker_owner_in_the_preimage() {
    let signer = signer();
    let owner = [13u8; 32];
    let bind = binding(100, &[]);
    let builder = bind
        .quote()
        .price(Price::new(2_500))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_123))
        .nonce(Nonce::new(9));

    let quote = builder
        .clone()
        .maker_owner(owner)
        .sign(&signer)
        .expect("signs");

    let server_view = OrderPreimageArgs {
        chain_id: 0,
        program_id: PROGRAM,
        is_taker_buy: false,
        position_type: PositionType::CoveredCall,
        market: MARKET,
        strike: quote.strike.value(),
        quantity: 5,
        gross_price: quote.price.value(),
        valid_until: quote.valid_until.as_unix_seconds(),
        maker: owner,
        taker: TAKER,
        nonce: quote.nonce.value(),
    };
    assert_eq!(compute_order_id(&server_view), *quote.order_id.as_bytes());
    assert_ne!(
        quote.order_id,
        builder.sign(&signer).expect("signs").order_id
    );

    // preimage_args agrees with the sign paths: a set owner wins over the
    // signer-key fallback argument.
    let args = builder
        .clone()
        .maker_owner(owner)
        .preimage_args(signer.pubkey_bytes())
        .expect("preimage");
    assert_eq!(compute_order_id(&args), *quote.order_id.as_bytes());

    let signature = crate::wire::decode_base58_64(&quote.signature).expect("base58 signature");
    verify_order_id_signature_bytes(
        quote.order_id.as_bytes(),
        &signature,
        &signer.pubkey_bytes(),
    )
    .expect("signature matches the delegated key");
}

#[tokio::test]
async fn delegated_async_signer_matches_the_local_path() {
    let remote = RemoteSigner(signer());
    let owner = [13u8; 32];
    let bind = binding(100, &[]);
    let builder = bind
        .quote()
        .price(Price::new(11))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(5))
        .maker_owner(owner);

    let quote = builder
        .sign_with_async_signer(&remote)
        .await
        .expect("remote signs");
    assert_eq!(
        quote.order_id,
        builder.sign(&signer()).expect("signs").order_id
    );
}
