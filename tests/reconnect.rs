use std::time::Duration;

use acta_maker_sdk::ws::reconnect::{
    jittered_reconnect_delay, jittered_reconnect_delay_with_ratio, next_reconnect_delay,
};

#[test]
fn next_delay_doubles() {
    let d1 = Duration::from_secs(1);
    let d2 = next_reconnect_delay(d1, Duration::from_secs(60));
    assert_eq!(d2, Duration::from_secs(2));

    let d3 = next_reconnect_delay(d2, Duration::from_secs(60));
    assert_eq!(d3, Duration::from_secs(4));
}

#[test]
fn next_delay_caps_at_max() {
    let max = Duration::from_secs(10);
    let d = next_reconnect_delay(Duration::from_secs(8), max);
    assert_eq!(d, max);

    let d2 = next_reconnect_delay(max, max);
    assert_eq!(d2, max);
}

#[test]
fn jittered_delay_within_bounds() {
    let base = Duration::from_secs(10);
    for _ in 0..20 {
        let jittered = jittered_reconnect_delay(base);
        let ms = jittered.as_millis();
        assert!(ms >= 8000, "jittered {ms}ms < 8000ms");
        assert!(ms <= 12001, "jittered {ms}ms > 12001ms");
    }
}

#[test]
fn zero_jitter_keeps_the_exact_delay() {
    let base = Duration::from_millis(137);
    assert_eq!(jittered_reconnect_delay_with_ratio(base, 0.0), base);
}
