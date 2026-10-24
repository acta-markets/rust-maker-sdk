use super::*;
use serde_json::{Value, json};
use std::time::Duration;

fn tracker() -> QuoteOutcomeTracker {
    QuoteOutcomeTracker::new(
        Uuid::nil(),
        OrderId::new([1; 32]),
        7,
        Instant::now() + Duration::from_secs(60),
    )
}

fn message(kind: &str, extra: Value) -> ServerMessage {
    let mut data = json!({"rfq_id": Uuid::nil(), "order_id": OrderId::new([1; 32])});
    data.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(json!({"type": kind, "data": data})).unwrap()
}

fn feed(t: &mut QuoteOutcomeTracker, kind: &str, extra: Value) {
    t.observe_message(7, Instant::now(), &message(kind, extra));
}

fn selected() -> Value {
    json!({"taker":"taker", "price":1, "quantity":1, "strike":1, "signature_deadline":1})
}

fn filled() -> Value {
    json!({"market_pda":"market", "position_type":"covered_call", "taker":"taker", "price":1, "quantity":1, "strike":1, "position_pda":"position", "tx_signature":"tx", "filled_at":1})
}

fn closed(reason: &str) -> Value {
    json!({"rfq_version":1, "reason":reason, "closed_at":1})
}

#[test]
fn acknowledged_selected_closed_then_filled() {
    let mut t = tracker();
    assert!(matches!(t.outcome(), QuoteOutcome::Pending));
    feed(&mut t, "QuoteAcknowledged", json!({}));
    assert!(matches!(t.outcome(), QuoteOutcome::Acked));
    feed(&mut t, "QuoteSelected", selected());
    assert!(matches!(t.outcome(), QuoteOutcome::Selected));
    feed(&mut t, "QuoteAcknowledged", json!({}));
    assert!(matches!(t.outcome(), QuoteOutcome::Selected));
    feed(&mut t, "RfqClosed", closed("filled"));
    assert!(!t.outcome().is_terminal());
    feed(&mut t, "QuoteFilled", filled());
    assert!(matches!(t.outcome(), QuoteOutcome::Filled(m) if m.tx_signature == "tx"));
    t.check(8, t.deadline);
    feed(&mut t, "QuoteExpired", json!({"reason":"late"}));
    assert!(matches!(t.outcome(), QuoteOutcome::Filled(_)));
}

#[test]
fn terminal_events_from_each_live_state() {
    for start in ["pending", "acked", "selected"] {
        for end in [
            "QuoteRejected",
            "QuoteExpired",
            "QuoteCancelled",
            "QuoteFilled",
            "RfqClosed",
        ] {
            let mut t = tracker();
            if start != "pending" {
                feed(&mut t, "QuoteAcknowledged", json!({}));
            }
            if start == "selected" {
                feed(&mut t, "QuoteSelected", selected());
            }
            let extra = match end {
                "QuoteRejected" => json!({"reason":"rate_limited"}),
                "QuoteExpired" => json!({"reason":"expired"}),
                "QuoteCancelled" => {
                    json!({"order_ids":[OrderId::new([1;32])], "reason":"requested", "cancelled_at":1})
                }
                "QuoteFilled" => filled(),
                _ => closed("taker_cancelled"),
            };
            feed(&mut t, end, extra);
            assert!(matches!(
                (end, t.outcome()),
                (
                    "QuoteRejected",
                    QuoteOutcome::Rejected(QuoteRejectReason::RateLimited)
                ) | ("QuoteExpired", QuoteOutcome::Expired)
                    | ("QuoteCancelled", QuoteOutcome::Cancelled)
                    | ("QuoteFilled", QuoteOutcome::Filled(_))
                    | (
                        "RfqClosed",
                        QuoteOutcome::Lost(QuoteLossReason::RfqClosed(
                            RfqCloseReason::TakerCancelled
                        ))
                    )
            ));
        }
    }
}

#[test]
fn closure_status_is_order_specific_and_filled_waits_for_details() {
    for (status, terminal) in [
        ("filled", false),
        ("unknown", false),
        ("outbid", true),
        ("expired", true),
        ("cancelled", true),
    ] {
        let mut t = tracker();
        let mut close = closed("filled");
        close["your_quote"] = json!({"order_id":t.order_id,"status":status,"price":1});
        feed(&mut t, "RfqClosed", close.clone());
        assert_eq!(t.outcome().is_terminal(), terminal);
        match status {
            "outbid" => assert!(matches!(
                t.outcome(),
                QuoteOutcome::Lost(QuoteLossReason::RfqClosed(RfqCloseReason::Filled))
            )),
            "expired" => assert!(matches!(t.outcome(), QuoteOutcome::Expired)),
            "cancelled" => assert!(matches!(t.outcome(), QuoteOutcome::Cancelled)),
            _ => {
                feed(&mut t, "QuoteFilled", filled());
                assert!(matches!(t.outcome(), QuoteOutcome::Filled(_)));
            }
        }
        let mut t = tracker();
        close["your_quote"]["order_id"] = json!(OrderId::new([2; 32]));
        feed(&mut t, "RfqClosed", close);
        assert!(matches!(t.outcome(), QuoteOutcome::Pending));
    }
}

#[test]
fn unrelated_messages_and_old_epochs_are_ignored() {
    let mut t = tracker();
    feed(
        &mut t,
        "QuoteAcknowledged",
        json!({"order_id":OrderId::new([2;32])}),
    );
    feed(
        &mut t,
        "QuoteRejected",
        json!({"rfq_id":Uuid::new_v4(),"reason":"internal_error"}),
    );
    let mut close = closed("expired");
    close["rfq_id"] = json!(Uuid::new_v4());
    feed(&mut t, "RfqClosed", close);
    feed(
        &mut t,
        "QuoteCancelled",
        json!({"order_ids":[OrderId::new([2;32])],"reason":"requested","cancelled_at":1}),
    );
    t.observe_message(6, Instant::now(), &message("QuoteAcknowledged", json!({})));
    assert!(matches!(t.outcome(), QuoteOutcome::Pending));
}

#[test]
fn unknown_is_sticky_for_reconnect_timeout_and_stream_failure() {
    for start in ["pending", "acked", "selected"] {
        for reason in [
            QuoteUnknownReason::EpochChanged,
            QuoteUnknownReason::TimedOut,
            QuoteUnknownReason::StreamGap,
            QuoteUnknownReason::StreamClosed,
        ] {
            let mut t = tracker();
            if start != "pending" {
                feed(&mut t, "QuoteAcknowledged", json!({}));
            }
            if start == "selected" {
                feed(&mut t, "QuoteSelected", selected());
            }
            match reason {
                QuoteUnknownReason::EpochChanged => {
                    t.check(8, Instant::now());
                }
                QuoteUnknownReason::TimedOut => {
                    t.check(7, t.deadline);
                }
                _ => {
                    t.mark_unknown(reason);
                }
            }
            feed(&mut t, "QuoteFilled", filled());
            assert!(matches!(t.outcome(), QuoteOutcome::Unknown(r) if *r == reason));
        }
    }
}

#[test]
fn inbound_epoch_and_receipt_deadline_are_checked() {
    let ack = message("QuoteAcknowledged", json!({}));
    let mut t = tracker();
    t.observe_message(8, Instant::now(), &ack);
    assert!(matches!(
        t.outcome(),
        QuoteOutcome::Unknown(QuoteUnknownReason::EpochChanged)
    ));
    let mut t = tracker();
    t.observe_message(7, t.deadline - Duration::from_nanos(1), &ack);
    assert!(matches!(t.outcome(), QuoteOutcome::Acked));
    t.observe_message(7, t.deadline, &message("QuoteFilled", filled()));
    assert!(matches!(
        t.outcome(),
        QuoteOutcome::Unknown(QuoteUnknownReason::TimedOut)
    ));
}

#[tokio::test]
async fn consumes_the_clients_message_stream() {
    use crate::ws::managed::ManagedWsHandle;
    let (handle, _commands) = ManagedWsHandle::test_handle(4, 4);
    let mut messages = handle.subscribe_messages();
    let mut t = QuoteOutcomeTracker::new(
        Uuid::nil(),
        OrderId::new([1; 32]),
        0,
        Instant::now() + Duration::from_secs(60),
    );
    handle.inject_message(message("QuoteAcknowledged", json!({})));
    t.observe(&messages.recv().await.unwrap());
    assert!(matches!(t.outcome(), QuoteOutcome::Acked));
    handle.inject_message(message("QuoteFilled", filled()));
    t.observe(&messages.recv().await.unwrap());
    assert!(matches!(t.outcome(), QuoteOutcome::Filled(_)));
}

#[test]
fn accepting_another_quote_is_a_loss() {
    let mut t = tracker();
    feed(&mut t, "QuoteAcknowledged", json!({}));
    feed(
        &mut t,
        "QuoteCancelled",
        json!({"order_ids":[OrderId::new([1;32])], "reason":"rfq_accepted", "cancelled_at":1}),
    );
    assert!(matches!(
        t.outcome(),
        QuoteOutcome::Lost(QuoteLossReason::RfqAccepted)
    ));
}

#[test]
fn failed_selection_returns_to_acked_and_can_be_selected_again() {
    let mut t = tracker();
    feed(&mut t, "QuoteSelected", selected());
    feed(
        &mut t,
        "RfqAvailableAgain",
        json!({"rfq_id":Uuid::new_v4(), "rfq_version":2, "reason":"tx_failed", "available_again_at":1}),
    );
    assert!(matches!(t.outcome(), QuoteOutcome::Selected));
    feed(
        &mut t,
        "RfqAvailableAgain",
        json!({"rfq_version":2, "reason":"tx_failed", "available_again_at":1}),
    );
    assert!(matches!(t.outcome(), QuoteOutcome::Acked));
    feed(&mut t, "QuoteSelected", selected());
    feed(&mut t, "QuoteFilled", filled());
    assert!(matches!(t.outcome(), QuoteOutcome::Filled(_)));
}

#[test]
fn acknowledged_but_unselected_order_loses_when_rfq_fills() {
    for personalized in [false, true] {
        let mut t = tracker();
        feed(&mut t, "QuoteAcknowledged", json!({}));
        let mut close = closed("filled");
        if personalized {
            close["your_quote"] =
                json!({"order_id":OrderId::new([2;32]),"status":"filled","price":1});
        }
        feed(&mut t, "RfqClosed", close);
        assert!(matches!(
            t.outcome(),
            QuoteOutcome::Lost(QuoteLossReason::RfqClosed(RfqCloseReason::Filled))
        ));
    }
}
