use super::*;

const FRACTIONAL: u64 = 1_800_000_000_700;

#[test]
fn system_time_conversion_truncates_like_the_preimage() {
    let time = UNIX_EPOCH + Duration::from_millis(FRACTIONAL);
    let expiry = QuoteExpiry::from_system_time(time).expect("in range");

    assert_eq!(expiry.as_unix_seconds(), 1_800_000_000);
    assert_eq!(
        expiry.as_unix_seconds(),
        time.duration_since(UNIX_EPOCH)
            .expect("after epoch")
            .as_secs(),
        "conversion must agree with the preimage's own truncation"
    );
}

#[test]
fn json_is_a_bare_integer_of_whole_seconds() {
    let expiry = QuoteExpiry::from_system_time(UNIX_EPOCH + Duration::from_millis(FRACTIONAL))
        .expect("in range");

    let json = serde_json::to_string(&expiry).expect("serializes");
    assert_eq!(json, "1800000000");

    let decoded: QuoteExpiry = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(decoded, expiry);
}

#[test]
fn round_trip_through_system_time_is_stable() {
    let expiry = QuoteExpiry::from_unix_seconds(1_700_000_123);
    let back = QuoteExpiry::from_system_time(expiry.to_system_time()).expect("in range");
    assert_eq!(back, expiry);
}

#[test]
fn times_before_the_epoch_are_rejected() {
    assert_eq!(
        QuoteExpiry::from_system_time(UNIX_EPOCH - Duration::from_secs(1)),
        None
    );
}

#[test]
fn remaining_is_none_once_the_expiry_has_passed() {
    let expiry = QuoteExpiry::from_unix_seconds(1_000);
    let now = UNIX_EPOCH + Duration::from_secs(1_001);
    assert_eq!(expiry.remaining_from(now), None);
    assert_eq!(
        expiry.remaining_from(UNIX_EPOCH + Duration::from_secs(900)),
        Some(Duration::from_secs(100))
    );
}

#[test]
fn after_produces_a_whole_second_expiry() {
    let expiry = QuoteExpiry::after(Duration::from_secs(300)).expect("clock after epoch");
    let seconds = expiry
        .to_system_time()
        .duration_since(UNIX_EPOCH)
        .expect("after epoch");
    assert_eq!(seconds.subsec_nanos(), 0);
}
