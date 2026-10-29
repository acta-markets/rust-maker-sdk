//! `QuoteExpiry` must match backend `UnixSecondsU64`: truncate on serialization,
//! read a bare u64. Drift breaks order preimage verification.

use acta_maker_sdk::QuoteExpiry;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{OptionTerms, TakerSide};

#[test]
fn server_frames_parse_with_a_bare_integer_expiry() {
    let quote_received = r#"{"type":"QuoteReceived","data":{
        "rfq_id":"6f0a4b6e-0000-4000-8000-000000000001",
        "terms":{"type":"covered_call","strike":100000000000},"side":"sell",
        "maker":"d4Sdh8BP11q3E6NbEi1Qrn9DbPXg2XKZnX2Pcvemge1","price":5000000,
        "valid_until":1800000123,"nonce":7,
        "order_id":"00000000000000000000000000000000000000000000000000000000000000aa",
        "net_price":null}}"#;
    match parse_server_message(quote_received).expect("parses") {
        ServerMessage::QuoteReceived(msg) => {
            assert_eq!(
                msg.valid_until,
                QuoteExpiry::from_unix_seconds(1_800_000_123)
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }

    let refresh = r#"{"type":"QuoteRefreshRequested","data":{
        "rfq_id":"6f0a4b6e-0000-4000-8000-000000000002",
        "terms":{"type":"covered_call","strike":100000000000},"side":"sell",
        "min_valid_until":1800000456,"reason":"expiring_soon"}}"#;
    match parse_server_message(refresh).expect("parses") {
        ServerMessage::QuoteRefreshRequested(msg) => {
            assert_eq!(
                msg.min_valid_until,
                QuoteExpiry::from_unix_seconds(1_800_000_456)
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// An unknown message type must not be produced by a parse failure on a known
/// one: if the expiry encoding regressed, this would silently become `Unknown`.
#[test]
fn a_known_frame_parses_as_its_type() {
    let quote_received = r#"{"type":"QuoteReceived","data":{
        "rfq_id":"6f0a4b6e-0000-4000-8000-000000000001",
        "terms":{"type":"covered_call","strike":1},"side":"sell","maker":"7eaSCSQv2PN6eFnRhv8ozsYN89yoPY7GKZUVa7DSDfqs","price":1,
        "valid_until":1800000123,"nonce":1,
        "order_id":"00000000000000000000000000000000000000000000000000000000000000aa",
        "net_price":null}}"#;
    assert!(matches!(
        parse_server_message(quote_received).expect("parses"),
        ServerMessage::QuoteReceived(_)
    ));
}

/// The outbound direction: the backend reads `valid_until` with `u64::deserialize`,
/// so we must emit a bare integer with no fractional part and no string wrapper.
#[test]
fn outbound_quote_emits_a_bare_integer_expiry() {
    use acta_maker_sdk::{Nonce, OrderId, Price};

    let quote = QuoteMessage {
        rfq_id: uuid::Uuid::nil(),
        terms: OptionTerms::CoveredCall { strike: 1.into() },
        side: TakerSide::Sell,
        price: Price::new(2),
        valid_until: QuoteExpiry::from_unix_seconds(1_800_000_123),
        nonce: Nonce::new(3),
        order_id: OrderId::new([0xaa; 32]),
        signature: "sig".to_string(),
    };
    let json = serde_json::to_string(&quote).expect("serializes");
    assert!(
        json.contains(r#""valid_until":1800000123"#),
        "expiry must be a bare integer, got: {json}"
    );
}
