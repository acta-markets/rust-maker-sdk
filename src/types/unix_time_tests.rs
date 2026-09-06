use super::*;

use serde_with::serde_as;

#[serde_as]
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Secs(#[serde_as(as = "UnixSeconds")] SystemTime);

#[serde_as]
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Millis(#[serde_as(as = "UnixMillis")] SystemTime);

#[test]
fn seconds_truncate_rather_than_round() {
    let late = Secs(UNIX_EPOCH + Duration::from_millis(1_800_000_000_700));
    assert_eq!(serde_json::to_string(&late).expect("ser"), "1800000000");

    let early = Secs(UNIX_EPOCH + Duration::from_millis(1_800_000_000_400));
    assert_eq!(serde_json::to_string(&early).expect("ser"), "1800000000");
}

#[test]
fn milliseconds_truncate_rather_than_round() {
    let late = Millis(UNIX_EPOCH + Duration::from_micros(1_800_000_000_700_900));
    assert_eq!(serde_json::to_string(&late).expect("ser"), "1800000000700");
}

#[test]
fn whole_values_round_trip() {
    let value = Secs(UNIX_EPOCH + Duration::from_secs(1_700_000_123));
    let json = serde_json::to_string(&value).expect("ser");
    assert_eq!(serde_json::from_str::<Secs>(&json).expect("de"), value);
}

#[test]
fn a_negative_timestamp_is_refused_like_the_backend() {
    assert!(serde_json::from_str::<Secs>("-1").is_err());
}

#[test]
fn times_before_the_epoch_cannot_be_serialized() {
    assert!(serde_json::to_string(&Secs(UNIX_EPOCH - Duration::from_secs(1))).is_err());
}
