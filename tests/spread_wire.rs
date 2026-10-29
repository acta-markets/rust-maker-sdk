use acta_maker_sdk::ws::types::{ClientMessage, ServerMessage};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

const SPREADS_WIRE: &[u8] = include_bytes!("fixtures/spreads_wire.json");

fn frames() -> serde_json::Map<String, Value> {
    serde_json::from_slice(SPREADS_WIRE).expect("vendored backend wire fixture")
}

#[test]
fn vendored_wire_fixture_matches_the_backend_when_the_sibling_exists() {
    let canonical = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rust-backend/acta-ws-protocol/tests/fixtures/spreads_wire.json");
    if canonical.is_file() {
        let canonical_bytes =
            std::fs::read(&canonical).expect("read canonical backend wire fixture");
        assert_eq!(
            Sha256::digest(&canonical_bytes),
            Sha256::digest(SPREADS_WIRE),
            "vendored fixture checksum differs from {}",
            canonical.display()
        );
        assert_eq!(canonical_bytes.as_slice(), SPREADS_WIRE);
    }
}

#[test]
fn backend_wire_frames_round_trip_without_losing_terms_side_or_nullable_net_price() {
    for (name, value) in frames() {
        let actual = match name.as_str() {
            "RfqRequest" | "PutRfqRequest" | "Quote" | "ReplaceQuote" | "CheckQuote" => {
                serde_json::to_value(
                    serde_json::from_value::<ClientMessage>(value.clone())
                        .expect("client fixture frame parses"),
                )
            }
            _ => serde_json::to_value(
                serde_json::from_value::<ServerMessage>(value.clone())
                    .expect("server fixture frame parses"),
            ),
        }
        .expect("fixture frame serializes");
        assert_eq!(actual, value, "{name}");
    }
}

#[test]
fn terms_and_side_are_required_on_every_strict_client_frame() {
    let frames = frames();
    for name in [
        "RfqRequest",
        "PutRfqRequest",
        "Quote",
        "ReplaceQuote",
        "CheckQuote",
    ] {
        for field in ["terms", "side"] {
            let mut value = frames[name].clone();
            value["data"]
                .as_object_mut()
                .expect("fixture data object")
                .remove(field);
            assert!(
                serde_json::from_value::<ClientMessage>(value).is_err(),
                "{name}.{field} must be required"
            );
        }
    }
}

#[test]
fn strict_terms_reject_mixed_or_unknown_legs() {
    let frames = frames();
    let mut mixed = frames["RfqRequest"].clone();
    mixed["data"]["terms"]
        .as_object_mut()
        .expect("terms object")
        .insert("strike".into(), 100_000_000_000_u64.into());
    assert!(serde_json::from_value::<ClientMessage>(mixed).is_err());

    let mut physical_with_upper = frames["RfqRequest"].clone();
    physical_with_upper["data"]["terms"] = serde_json::json!({
        "type": "covered_call",
        "strike": 100_000_000_000_u64,
        "upper_strike": 120_000_000_000_u64,
    });
    assert!(serde_json::from_value::<ClientMessage>(physical_with_upper).is_err());
}
