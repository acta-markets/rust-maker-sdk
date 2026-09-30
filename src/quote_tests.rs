use super::*;

use crate::orders::{BytesSigner, verify_order_id_signature_bytes};
use crate::signing::{AsyncSignerLike, SigningFuture};
use crate::types::ids::ChainId;
use crate::ws::types::{MarketDescriptor, RfqOrderOption};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROGRAM: [u8; 32] =
    solana_pubkey::pubkey!("33Ezs5eoa16QyPW8wifnyz2nCyMEcq2crqkVBNjnTE8U").to_bytes();
const TAKER: [u8; 32] = [11u8; 32];

fn market() -> [u8; 32] {
    solana_pubkey::pubkey!("6E9r7Y4Q7mVpNDUJ47dYAdNQAZN1u3FxrutUWAfjL4zb").to_bytes()
}

fn signer() -> BytesSigner {
    BytesSigner::from_secret([3u8; 32])
}

fn broadcast(strike: u64, alternatives: &[u64]) -> RfqBroadcastMessage {
    RfqBroadcastMessage {
        rfq_id: Uuid::from_u128(42),
        market: MarketDescriptor {
            chain_id: ChainId::new(0),
            program_id: encode_base58(&PROGRAM),
            market_pda: encode_base58(&market()),
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
        market: market(),
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

struct RemoteSigner(BytesSigner);

impl AsyncSignerLike for RemoteSigner {
    fn pubkey_bytes(&self) -> [u8; 32] {
        self.0.pubkey_bytes()
    }

    fn sign_message<'a>(&'a self, message: &'a [u8]) -> SigningFuture<'a> {
        Box::pin(async move { Ok(self.0.sign_message(message)) })
    }
}

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
        market: market(),
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

#[test]
fn put_binding_requires_the_locally_selected_program_and_seed_terms() {
    let custom_program = [4; 32];
    let mut rfq = broadcast(100, &[]);
    rfq.market.program_id = encode_base58(&custom_program);
    rfq.market.is_put = true;
    rfq.position_type = PositionType::CashSecuredPut;
    rfq.market.collateral_mint = rfq.market.quote_mint.clone();
    rfq.market.settlement_mint = rfq.market.underlying_mint.clone();
    let pda = solana_pubkey::Pubkey::find_program_address(
        &[
            b"market",
            &[1; 32],
            &[2; 32],
            &2_000_000_000u64.to_le_bytes(),
            &[1],
        ],
        &solana_pubkey::Pubkey::new_from_array(custom_program),
    )
    .0
    .to_bytes();
    rfq.market.market_pda = encode_base58(&pda);
    assert!(matches!(
        RfqBinding::from_broadcast(&rfq),
        Err(QuoteBuildError::MarketBinding(_))
    ));
    let binding = RfqBinding::from_broadcast_for_program(&rfq, custom_program)
        .expect("configured deployment");
    let preimage = binding
        .quote()
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(1))
        .preimage_args(TAKER)
        .expect("preimage");
    assert_eq!(preimage.program_id, custom_program);
    assert_eq!(preimage.market, pda);
    assert_eq!(preimage.position_type, PositionType::CashSecuredPut);
    rfq.market.expiry_ts += Duration::from_millis(1);
    assert!(RfqBinding::from_broadcast_for_program(&rfq, custom_program).is_err());
}

#[test]
fn broadcast_binding_rejects_descriptor_substitution() {
    let valid = broadcast(100, &[]);
    assert!(RfqBinding::from_broadcast(&valid).is_ok());
    for field in [
        "program",
        "chain",
        "market",
        "underlying",
        "quote",
        "expiry",
        "is_put",
        "position_type",
        "collateral",
        "settlement",
    ] {
        let mut rfq = valid.clone();
        match field {
            "program" => rfq.market.program_id = encode_base58(&[4; 32]),
            "chain" => rfq.market.chain_id = ChainId::new(1),
            "market" => rfq.market.market_pda = encode_base58(&[4; 32]),
            "underlying" => rfq.market.underlying_mint = encode_base58(&[4; 32]),
            "quote" => rfq.market.quote_mint = encode_base58(&[4; 32]),
            "expiry" => rfq.market.expiry_ts += Duration::from_secs(1),
            "is_put" => rfq.market.is_put = true,
            "position_type" => rfq.position_type = PositionType::CashSecuredPut,
            "collateral" => rfq.market.collateral_mint = encode_base58(&[4; 32]),
            "settlement" => rfq.market.settlement_mint = encode_base58(&[4; 32]),
            _ => unreachable!(),
        }
        assert!(
            RfqBinding::from_broadcast(&rfq).is_err(),
            "accepted substituted {field}"
        );
    }
}
