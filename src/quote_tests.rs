use super::*;

use crate::orders::{BytesSigner, verify_order_id_signature_bytes};
use crate::signing::{AsyncSignerLike, SigningFuture};
use crate::types::ids::{ChainId, Strike};
use crate::ws::types::MarketDescriptor;
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

fn covered_call(strike: u64) -> OptionTerms {
    OptionTerms::CoveredCall {
        strike: Strike::new(strike),
    }
}

fn broadcast(terms: OptionTerms, alternatives: &[OptionTerms]) -> RfqBroadcastMessage {
    RfqBroadcastMessage {
        rfq_id: Uuid::from_u128(42),
        market: MarketDescriptor {
            chain_id: ChainId::new(0),
            program_id: crate::Address32::new(PROGRAM),
            market_pda: crate::Address32::new(market()),
            underlying_mint: crate::Address32::new([1u8; 32]),
            quote_mint: crate::Address32::new([2u8; 32]),
            expiry_ts: UNIX_EPOCH + Duration::from_secs(2_000_000_000),
            is_put: false,
            collateral_mint: crate::Address32::new([1u8; 32]),
            settlement_mint: crate::Address32::new([2u8; 32]),
        },
        terms,
        side: TakerSide::Sell,
        quantity: Quantity::new(5),
        expires_at: UNIX_EPOCH + Duration::from_secs(1_800_000_300),
        taker: crate::Address32::new(TAKER),
        sent_at_unix_ms: std::time::UNIX_EPOCH,
        order_options: alternatives.to_vec(),
    }
}

fn binding(strike: u64, alternatives: &[u64]) -> RfqBinding {
    let alternatives = alternatives
        .iter()
        .copied()
        .map(covered_call)
        .collect::<Vec<_>>();
    RfqBinding::from_broadcast(&broadcast(covered_call(strike), &alternatives))
        .expect("valid identities")
}

fn quote_on(binding: &RfqBinding) -> QuoteBuilder<'_> {
    binding
        .quote()
        .maker_owner(crate::Address32::new(signer().pubkey_bytes()))
        .min_valid_until(QuoteExpiry::from_unix_seconds(1))
}

#[test]
fn wire_valid_until_is_the_value_that_was_signed() {
    let fractional = SystemTime::UNIX_EPOCH + Duration::from_millis(1_800_000_000_700);
    let expiry = QuoteExpiry::from_system_time(fractional).expect("in range");
    let signer = signer();
    let builder_binding = binding(100, &[]);
    let builder = quote_on(&builder_binding)
        .price(Price::new(1_000))
        .valid_until(expiry)
        .nonce(Nonce::new(77));

    let args = builder.preimage_args().expect("resolves");
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
    let quote = quote_on(&bind)
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
        side: TakerSide::Sell,
        position_type: decoded.terms.position_type(),
        market: market(),
        strike: decoded.terms.strike().value(),
        quantity: 5,
        gross_price: decoded.price.value(),
        valid_until: decoded.valid_until.as_unix_seconds(),
        maker: signer.pubkey_bytes(),
        taker: TAKER,
        nonce: decoded.nonce.value(),
    };

    assert_eq!(
        compute_order_id(&server_view).unwrap(),
        *decoded.order_id.as_bytes()
    );
}

#[test]
fn signature_verifies_against_the_order_id() {
    let signer = signer();
    let bind = binding(100, &[]);
    let quote = quote_on(&bind)
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
fn alternative_terms_from_order_options_are_accepted() {
    let bind = binding(100, &[110, 120]);
    assert_eq!(
        bind.offered_terms(),
        &[covered_call(100), covered_call(110), covered_call(120)]
    );

    let quote = quote_on(&bind)
        .terms(covered_call(120))
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect("signs");

    assert_eq!(quote.terms, covered_call(120));
}

#[test]
fn terms_the_rfq_did_not_offer_are_refused_locally() {
    let bind = binding(100, &[110]);
    let error = quote_on(&bind)
        .terms(covered_call(999))
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("unlisted strike");

    assert!(matches!(
        error,
        QuoteBuildError::TermsNotOffered { requested, .. } if requested == covered_call(999)
    ));
}

#[test]
fn duplicate_order_option_does_not_repeat_the_requested_terms() {
    let bind = binding(100, &[100, 110]);
    assert_eq!(
        bind.offered_terms(),
        &[covered_call(100), covered_call(110)]
    );
}

#[test]
fn missing_fields_are_named() {
    let bind = binding(100, &[]);
    let missing_price = quote_on(&bind)
        .valid_until(QuoteExpiry::from_unix_seconds(1))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("no price");
    assert!(matches!(missing_price, QuoteBuildError::Missing("price")));

    let missing_expiry = quote_on(&bind)
        .price(Price::new(1))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("no expiry");
    assert!(matches!(
        missing_expiry,
        QuoteBuildError::Missing("valid_until")
    ));

    let missing_nonce = quote_on(&bind)
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1))
        .sign(&signer())
        .expect_err("no nonce");
    assert!(matches!(missing_nonce, QuoteBuildError::Missing("nonce")));

    let complete = quote_on(&bind)
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1))
        .nonce(Nonce::new(1));
    let missing_owner = bind
        .quote()
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1))
        .min_valid_until(QuoteExpiry::from_unix_seconds(1))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("no owner");
    assert!(matches!(
        missing_owner,
        QuoteBuildError::Missing("maker_owner")
    ));
    let missing_floor = bind
        .quote()
        .maker_owner(crate::Address32::new(signer().pubkey_bytes()))
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1))
        .nonce(Nonce::new(1))
        .sign(&signer())
        .expect_err("no floor");
    assert!(matches!(
        missing_floor,
        QuoteBuildError::Missing("min_valid_until")
    ));
    assert!(complete.sign(&signer()).is_ok());
}

#[test]
fn valid_until_must_sit_between_the_venue_floor_and_market_expiry() {
    let bind = binding(100, &[]);
    let market_expiry = QuoteExpiry::from_unix_seconds(2_000_000_000);
    assert_eq!(bind.market_expiry(), market_expiry);
    let builder = quote_on(&bind)
        .price(Price::new(1))
        .nonce(Nonce::new(1))
        .min_valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000));

    assert!(
        builder
            .clone()
            .valid_until(market_expiry)
            .sign(&signer())
            .is_ok()
    );
    assert!(
        builder
            .clone()
            .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
            .sign(&signer())
            .is_ok()
    );
    assert!(matches!(
        builder
            .clone()
            .valid_until(QuoteExpiry::from_unix_seconds(2_000_000_001))
            .sign(&signer()),
        Err(QuoteBuildError::ValidUntilAfterMarketExpiry { .. })
    ));
    assert!(matches!(
        builder
            .valid_until(QuoteExpiry::from_unix_seconds(1_799_999_999))
            .preimage_args(),
        Err(QuoteBuildError::ValidUntilTooSoon { .. })
    ));
}

#[cfg(feature = "http")]
#[test]
fn venue_sets_the_floor_and_checks_the_settlement_buffer() {
    let venue = crate::http::Venue {
        protocol_fee_bps_premium: 100,
        protocol_fee_bps_volume: 10,
        program_id: crate::Address32::new(PROGRAM),

        min_quote_validity_seconds: 5,
        settlement_buffer_seconds: 30,
        quote_refresh_lead_seconds: 3,
        min_rfq_timeout_seconds: 1,
        max_rfq_timeout_seconds: 60,
        signature_timeout_seconds: 10,
        max_quotes_per_rfq: crate::http::CountLimit::Unlimited {},
        max_active_rfqs_per_taker: crate::http::CountLimit::Unlimited {},
        pre_expiry_cutoff_seconds: 0,
    };
    let now = QuoteExpiry::after(Duration::ZERO)
        .expect("clock after epoch")
        .as_unix_seconds();
    let bind = binding(100, &[]);
    let builder = bind
        .quote()
        .maker_owner(crate::Address32::new(signer().pubkey_bytes()))
        .price(Price::new(1))
        .nonce(Nonce::new(1))
        .venue(&venue);

    assert!(matches!(
        builder
            .clone()
            .valid_until(QuoteExpiry::from_unix_seconds(now + 4))
            .preimage_args(),
        Err(QuoteBuildError::ValidUntilTooSoon { .. })
    ));
    assert!(matches!(
        builder
            .clone()
            .valid_until(QuoteExpiry::from_unix_seconds(now + 20))
            .preimage_args(),
        Err(QuoteBuildError::ValidUntilWithinSettlementBuffer { .. })
    ));
    assert!(
        builder
            .valid_until(QuoteExpiry::from_unix_seconds(now + 60))
            .sign(&signer())
            .is_ok()
    );
}

#[test]
fn a_malformed_identity_names_its_field() {
    let rfq = broadcast(covered_call(100), &[]);
    let mut json = serde_json::to_value(rfq).expect("fixture");
    json["taker"] = serde_json::json!("not-base58-!!");
    assert!(serde_json::from_value::<crate::ws::types::RfqBroadcastMessage>(json).is_err());
}

#[test]
fn a_replacement_carries_the_same_signed_identity() {
    let signer = signer();
    let bind = binding(100, &[]);
    let builder = quote_on(&bind)
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
    let builder = quote_on(&bind)
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
    let builder = quote_on(&bind)
        .price(Price::new(2_500))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_123))
        .nonce(Nonce::new(9));

    let quote = builder
        .clone()
        .maker_owner(crate::Address32::new(owner))
        .sign(&signer)
        .expect("signs");

    let server_view = OrderPreimageArgs {
        chain_id: 0,
        program_id: PROGRAM,
        side: TakerSide::Sell,
        position_type: PositionType::CoveredCall,
        market: market(),
        strike: quote.terms.strike().value(),
        quantity: 5,
        gross_price: quote.price.value(),
        valid_until: quote.valid_until.as_unix_seconds(),
        maker: owner,
        taker: TAKER,
        nonce: quote.nonce.value(),
    };
    assert_eq!(
        compute_order_id(&server_view).unwrap(),
        *quote.order_id.as_bytes()
    );
    assert_ne!(
        quote.order_id,
        builder.sign(&signer).expect("signs").order_id
    );

    let args = builder
        .clone()
        .maker_owner(crate::Address32::new(owner))
        .preimage_args()
        .expect("preimage");
    assert_eq!(compute_order_id(&args).unwrap(), *quote.order_id.as_bytes());

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
    let builder = quote_on(&bind)
        .price(Price::new(11))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(5))
        .maker_owner(crate::Address32::new(owner));

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
    let mut rfq = broadcast(covered_call(100), &[]);
    rfq.market.program_id = crate::Address32::new(custom_program);
    rfq.market.is_put = true;
    rfq.terms = OptionTerms::CashSecuredPut {
        strike: rfq.terms.strike(),
    };
    rfq.market.collateral_mint = rfq.market.quote_mint;
    rfq.market.settlement_mint = rfq.market.underlying_mint;
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
    rfq.market.market_pda = crate::Address32::new(pda);
    assert!(matches!(
        RfqBinding::from_broadcast(&rfq),
        Err(QuoteBuildError::MarketBinding(_))
    ));
    let binding = RfqBinding::from_broadcast_for_program(&rfq, custom_program)
        .expect("configured deployment");
    let preimage = quote_on(&binding)
        .price(Price::new(1))
        .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
        .nonce(Nonce::new(1))
        .preimage_args()
        .expect("preimage");
    assert_eq!(preimage.program_id, custom_program);
    assert_eq!(preimage.market, pda);
    assert_eq!(preimage.position_type, PositionType::CashSecuredPut);
    rfq.market.expiry_ts += Duration::from_millis(1);
    assert!(RfqBinding::from_broadcast_for_program(&rfq, custom_program).is_err());
}

#[test]
fn broadcast_binding_rejects_descriptor_substitution() {
    let valid = broadcast(covered_call(100), &[]);
    assert!(RfqBinding::from_broadcast(&valid).is_ok());
    for field in [
        "program",
        "chain",
        "market",
        "underlying",
        "quote",
        "expiry",
        "is_put",
        "terms",
        "collateral",
        "settlement",
    ] {
        let mut rfq = valid.clone();
        match field {
            "program" => rfq.market.program_id = crate::Address32::new([4; 32]),
            "chain" => rfq.market.chain_id = ChainId::new(1),
            "market" => rfq.market.market_pda = crate::Address32::new([4; 32]),
            "underlying" => rfq.market.underlying_mint = crate::Address32::new([4; 32]),
            "quote" => rfq.market.quote_mint = crate::Address32::new([4; 32]),
            "expiry" => rfq.market.expiry_ts += Duration::from_secs(1),
            "is_put" => rfq.market.is_put = true,
            "terms" => {
                rfq.terms = OptionTerms::CashSecuredPut {
                    strike: rfq.terms.strike(),
                }
            }
            "collateral" => rfq.market.collateral_mint = crate::Address32::new([4; 32]),
            "settlement" => rfq.market.settlement_mint = crate::Address32::new([4; 32]),
            _ => unreachable!(),
        }
        assert!(
            RfqBinding::from_broadcast(&rfq).is_err(),
            "accepted substituted {field}"
        );
    }
}

#[test]
fn spread_quotes_bind_pair_and_sign_v2() {
    for position_type in [PositionType::CallSpread, PositionType::PutSpread] {
        let terms = match position_type {
            PositionType::CallSpread => OptionTerms::CallSpread {
                lower_strike: Strike::new(100_000_000_000),
                upper_strike: Strike::new(120_000_000_000),
            },
            PositionType::PutSpread => OptionTerms::PutSpread {
                lower_strike: Strike::new(100_000_000_000),
                upper_strike: Strike::new(120_000_000_000),
            },
            _ => unreachable!(),
        };
        let mut rfq = broadcast(terms, &[]);
        rfq.quantity = Quantity::new(1_000_000);
        rfq.order_options = vec![terms];
        rfq.market.is_put = position_type.is_put();
        rfq.market.collateral_mint = rfq.market.quote_mint;
        rfq.market.settlement_mint = rfq.market.quote_mint;
        rfq.market.market_pda = market_pda(
            &solana_pubkey::Pubkey::new_from_array(PROGRAM),
            &solana_pubkey::Pubkey::new_from_array([1; 32]),
            &solana_pubkey::Pubkey::new_from_array([2; 32]),
            2_000_000_000,
            position_type.is_put(),
        )
        .into();
        assert!(RfqBinding::from_broadcast(&rfq).is_ok());
        let binding = RfqBinding::from_broadcast_for_program(&rfq, PROGRAM).unwrap();
        let builder = quote_on(&binding)
            .price(Price::new(1_000_000_000))
            .valid_until(QuoteExpiry::from_unix_seconds(1_800_000_000))
            .nonce(Nonce::new(u64::MAX));
        let quote = builder.sign(&signer()).unwrap();
        assert_eq!(quote.terms, terms);
        assert_eq!(quote.side, TakerSide::Sell);
        assert!(builder.preimage_args().is_err());
        assert_eq!(
            *quote.order_id.as_bytes(),
            crate::orders::compute_spread_order_id(&builder.spread_preimage_args().unwrap())
                .unwrap()
        );
        assert!(
            builder
                .clone()
                .terms(match position_type {
                    PositionType::CallSpread => OptionTerms::CallSpread {
                        lower_strike: Strike::new(110_000_000_000),
                        upper_strike: Strike::new(120_000_000_000),
                    },
                    PositionType::PutSpread => OptionTerms::PutSpread {
                        lower_strike: Strike::new(110_000_000_000),
                        upper_strike: Strike::new(120_000_000_000),
                    },
                    _ => unreachable!(),
                })
                .sign(&signer())
                .is_err()
        );
        assert!(
            builder
                .price(Price::new(20_000_000_000))
                .sign(&signer())
                .is_err()
        );
        let replaced = quote.into_replacement(OrderId::new([1; 32]));
        assert_eq!(replaced.terms, terms);
        rfq.order_options[0] = match position_type {
            PositionType::CallSpread => OptionTerms::CallSpread {
                lower_strike: Strike::new(100_000_000_000),
                upper_strike: Strike::new(130_000_000_000),
            },
            PositionType::PutSpread => OptionTerms::PutSpread {
                lower_strike: Strike::new(100_000_000_000),
                upper_strike: Strike::new(130_000_000_000),
            },
            _ => unreachable!(),
        };
        assert!(RfqBinding::from_broadcast_for_program(&rfq, PROGRAM).is_err());
    }
}

#[test]
fn buy_terms_are_refused_before_quote_assembly() {
    let mut rfq = broadcast(covered_call(100), &[]);
    rfq.side = TakerSide::Buy;

    assert!(matches!(
        RfqBinding::from_broadcast(&rfq),
        Err(QuoteBuildError::TakerBuyUnsupported)
    ));
}
