use std::time::Duration;

use super::ReconnectBackoff;

#[test]
fn reconnect_backoff_only_resets_after_a_stable_authenticated_session() {
    let mut backoff = ReconnectBackoff::new(Duration::from_millis(100), Duration::from_millis(400));

    assert_eq!(backoff.next_attempt(), (1, Duration::from_millis(100)));
    assert_eq!(backoff.next_attempt(), (2, Duration::from_millis(200)));
    assert!(backoff.limit_reached(2));

    assert!(!backoff.reset_after_stable_session(Duration::from_millis(399)));
    assert!(backoff.limit_reached(2));
    assert_eq!(backoff.next_attempt(), (3, Duration::from_millis(400)));

    assert!(backoff.reset_after_stable_session(Duration::from_millis(400)));
    assert!(!backoff.limit_reached(2));
    assert_eq!(backoff.next_attempt(), (1, Duration::from_millis(100)));
}
