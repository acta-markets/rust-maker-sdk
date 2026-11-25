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

/// Builds the wire frame an event of the old name now travels in.
fn message(kind: &str, extra: Value) -> ServerMessage {
    let mut extra = extra.as_object().unwrap().clone();
    let mut ids = json!({"rfq_id": Uuid::nil(), "order_id": OrderId::new([1; 32])});
    for key in ["rfq_id", "order_id"] {
        if let Some(value) = extra.remove(key) {
            let _ = ids.as_object_mut().unwrap().insert(key.into(), value);
        }
    }
    let extra = Value::Object(extra);
    let state = |tag: &str, defaults: Value| {
        let mut state = defaults;
        let fields = state.as_object_mut().unwrap();
        let _ = fields.insert("type".into(), json!(tag));
        if tag != "rejected" {
            let _ = fields.insert("rfq_version".into(), json!(1));
        }
        fields.extend(extra.as_object().unwrap().clone());
        let mut data = ids.clone();
        let _ = data.as_object_mut().unwrap().insert("state".into(), state);
        json!({"type": "QuoteUpdate", "data": data})
    };
    let frame = match kind {
        "QuoteAcknowledged" => state(
            "live",
            json!({"is_best": true, "best_price": {"type": "none"}}),
        ),
        "QuoteSelected" => state("selected", json!({})),
        "QuoteFilled" => state("filled", json!({})),
        "QuoteExpired" => state("expired", json!({})),
        "QuoteRejected" => state("rejected", json!({"message": "rejected"})),
        "QuoteLost" => state("lost", json!({})),
        "QuoteReleased" => state("released", json!({})),
        "QuoteRefresh" => state(
            "refresh_requested",
            json!({"terms": {"type": "covered_call", "strike": "1"}, "side": "sell", "min_valid_until": 1, "reason": "expiring_soon"}),
        ),
        "QuoteCancelled" => state("cancelled", json!({})),
        "ReplyAccepted" => json!({"type": "Reply", "data": {
            "request_id": Uuid::nil(),
            "result": {"type": "accepted"},
        }}),
        "RfqClosed" | "RfqAvailableAgain" => {
            let mut fields = extra.as_object().unwrap().clone();
            let version = fields.remove("rfq_version").unwrap_or(json!(1));
            let tag = if kind == "RfqClosed" {
                "closed"
            } else {
                "available"
            };
            let mut state = json!({"type": tag});
            if kind == "RfqClosed" {
                let object = state.as_object_mut().unwrap();
                let _ = object.insert("winner".into(), json!({"type": "none"}));
            }
            if let Some(at) = fields
                .remove("closed_at")
                .or_else(|| fields.remove("available_again_at"))
            {
                let _ = fields.insert("at".into(), at);
            }
            state.as_object_mut().unwrap().extend(fields);
            json!({"type": "RfqUpdate", "data": {"rfq_id": ids["rfq_id"], "rfq_version": version, "state": state}})
        }
        _ => {
            let mut data = ids.clone();
            data.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            json!({"type": kind, "data": data})
        }
    };
    serde_json::from_value(frame).unwrap()
}

fn feed(t: &mut QuoteOutcomeTracker, kind: &str, extra: Value) {
    t.observe_message(7, Instant::now(), &message(kind, extra));
}

fn selected() -> Value {
    json!({
        "taker_owner":"5DNHbugre3SXXogpZroA5SEsENdAd69AfNK5Z7ZCWr84", "price":"1", "quantity":"1",
        "terms":{"type":"covered_call","strike":"1"}, "side":"sell",
        "signature_deadline_at":1
    })
}

fn filled() -> Value {
    json!({
        "market_pda":"GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9", "terms":{"type":"covered_call","strike":"1"}, "side":"sell",
        "taker_owner":"5DNHbugre3SXXogpZroA5SEsENdAd69AfNK5Z7ZCWr84", "price":"1", "quantity":"1", "position_pda":"75NqraLLofiLUpNQg2wUPG6CJpxcaGEZUwuNrnAgvcTP",
        "transaction_signature":"tx", "filled_at":1
    })
}

fn closed(reason: &str) -> Value {
    json!({"rfq_version":1, "reason":reason, "closed_at":1})
}

#[test]
fn acknowledged_selected_then_filled() {
    let mut t = tracker();
    assert!(matches!(t.outcome(), QuoteOutcome::Pending));
    feed(&mut t, "QuoteAcknowledged", json!({}));
    assert!(matches!(t.outcome(), QuoteOutcome::Acked));
    feed(&mut t, "QuoteSelected", selected());
    assert!(matches!(t.outcome(), QuoteOutcome::Selected));
    feed(&mut t, "QuoteAcknowledged", json!({}));
    assert!(matches!(t.outcome(), QuoteOutcome::Selected));
    feed(&mut t, "RfqClosed", closed("filled"));
    assert!(
        !t.outcome().is_terminal(),
        "an RFQ close says nothing about this order"
    );
    feed(&mut t, "QuoteFilled", filled());
    assert!(matches!(t.outcome(), QuoteOutcome::Filled(m) if m.transaction_signature == "tx"));
    t.check(8, t.deadline);
    feed(&mut t, "QuoteExpired", json!({"reason":"expired"}));
    assert!(matches!(t.outcome(), QuoteOutcome::Filled(_)));
}

#[test]
fn every_terminal_update_ends_the_order_once() {
    for start in ["pending", "acked", "selected"] {
        for (end, extra) in [
            ("QuoteExpired", json!({"reason":"expired"})),
            ("QuoteRefresh", json!({})),
            ("QuoteLost", json!({"reason":"taker_cancelled"})),
            ("QuoteReleased", json!({"reason":"signature_timeout"})),
            ("QuoteCancelled", json!({"reason":"risk_check"})),
            ("QuoteFilled", filled()),
        ] {
            let mut t = tracker();
            if start != "pending" {
                feed(&mut t, "QuoteAcknowledged", json!({}));
            }
            if start == "selected" {
                feed(&mut t, "QuoteSelected", selected());
            }
            feed(&mut t, end, extra);
            assert!(
                matches!(
                    (end, t.outcome()),
                    ("QuoteExpired", QuoteOutcome::Expired)
                        | ("QuoteRefresh", QuoteOutcome::RefreshRequested { .. })
                        | (
                            "QuoteLost",
                            QuoteOutcome::Lost(QuoteLossReason::RfqClosed(
                                RfqCloseReason::TakerCancelled
                            ))
                        )
                        | (
                            "QuoteReleased",
                            QuoteOutcome::Released(RfqAvailableAgainReason::SignatureTimeout)
                        )
                        | ("QuoteCancelled", QuoteOutcome::Cancelled)
                        | ("QuoteFilled", QuoteOutcome::Filled(_))
                ),
                "{start} -> {end}: {:?}",
                t.outcome()
            );
            feed(&mut t, "QuoteAcknowledged", json!({}));
            assert!(t.outcome().is_terminal(), "{start} -> {end} stays ended");
        }
    }
}

#[test]
fn refresh_requested_carries_what_a_requote_needs() {
    let mut t = tracker();
    feed(&mut t, "QuoteAcknowledged", json!({}));
    feed(
        &mut t,
        "QuoteRefresh",
        json!({"min_valid_until": 1_700_000_060}),
    );
    assert!(matches!(
        t.outcome(),
        QuoteOutcome::RefreshRequested {
            terms: OptionTerms::CoveredCall { .. },
            side: TakerSide::Sell,
            min_valid_until,
            reason: QuoteExpiryReason::ExpiringSoon,
        } if *min_valid_until == QuoteExpiry::from_unix_seconds(1_700_000_060)
    ));
    assert!(t.outcome().is_terminal());
}

#[test]
fn a_reply_does_not_end_the_order() {
    let mut t = tracker();
    feed(&mut t, "QuoteAcknowledged", json!({}));
    feed(&mut t, "ReplyAccepted", json!({}));
    assert!(matches!(t.outcome(), QuoteOutcome::Acked));
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
        json!({"order_id":OrderId::new([2;32]),"reason":"requested"}),
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
fn rejected_ends_only_a_pending_order() {
    let mut t = tracker();
    feed(
        &mut t,
        "QuoteRejected",
        json!({"reason":"duplicate_order_id"}),
    );
    assert!(matches!(
        t.outcome(),
        QuoteOutcome::Rejected(QuoteRejectReason::DuplicateOrderId)
    ));
    feed(&mut t, "QuoteAcknowledged", json!({}));
    assert!(t.outcome().is_terminal());
}

#[test]
fn rejected_resubmission_keeps_a_live_or_selected_order() {
    let mut t = tracker();
    feed(&mut t, "QuoteAcknowledged", json!({}));
    feed(
        &mut t,
        "QuoteRejected",
        json!({"reason":"duplicate_order_id"}),
    );
    assert!(matches!(t.outcome(), QuoteOutcome::Acked));
    feed(&mut t, "QuoteSelected", selected());
    feed(&mut t, "QuoteRejected", json!({"reason":"rate_limited"}));
    assert!(matches!(t.outcome(), QuoteOutcome::Selected));
    feed(&mut t, "QuoteFilled", filled());
    assert!(matches!(t.outcome(), QuoteOutcome::Filled(_)));
}
