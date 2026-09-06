//! `valid_until` must stay byte-identical to what the backend emits and accepts.
//!
//! The backend uses `UnixSecondsU64`: it truncates to whole seconds on
//! serialize and reads a bare `u64` on deserialize. `QuoteExpiry` must match on
//! both directions, or quotes are rejected as `order_id preimage mismatch`.

use acta_maker_sdk::QuoteExpiry;
use acta_maker_sdk::ws::types::*;

/// Exactly the shape the backend emits, hand-written rather than round-tripped
/// through our own serializer, so a change on our side cannot hide itself.
#[test]
fn server_frames_parse_with_a_bare_integer_expiry() {
    let quote_received = r#"{"type":"QuoteReceived","data":{
        "rfq_id":"6f0a4b6e-0000-4000-8000-000000000001",
        "strike":100000000000,"maker":"MaKeR","price":5000000,
        "valid_until":1800000123,"nonce":7,
        "order_id":"00000000000000000000000000000000000000000000000000000000000000aa"}}"#;
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
        "strike":100000000000,"min_valid_until":1800000456,"reason":"stale"}}"#;
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
fn a_known_frame_never_degrades_to_unknown() {
    let quote_received = r#"{"type":"QuoteReceived","data":{
        "rfq_id":"6f0a4b6e-0000-4000-8000-000000000001",
        "strike":1,"maker":"m","price":1,"valid_until":1800000123,"nonce":1,
        "order_id":"00000000000000000000000000000000000000000000000000000000000000aa"}}"#;
    assert!(!matches!(
        parse_server_message(quote_received).expect("parses"),
        ServerMessage::Unknown(_)
    ));
}

/// The outbound direction: the backend reads `valid_until` with `u64::deserialize`,
/// so we must emit a bare integer with no fractional part and no string wrapper.
#[test]
fn outbound_quote_emits_a_bare_integer_expiry() {
    use acta_maker_sdk::types::ids::Strike;
    use acta_maker_sdk::{Nonce, OrderId, Price};

    let quote = QuoteMessage {
        rfq_id: uuid::Uuid::nil(),
        strike: Strike::new(1),
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
