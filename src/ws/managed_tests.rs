use std::sync::Arc;

use super::{SendAwaitError, normalize_maker_ws_url, tracker::AwaitTracker};
use crate::ws::types::common::WsChannel;
use crate::ws::types::{RequestErrorEnvelope, ServerError, ServerMessage, SubscribeAckData};
use tokio::sync::oneshot;
use uuid::Uuid;

#[test]
fn appends_maker_to_bare_url() {
    assert_eq!(
        normalize_maker_ws_url("ws://localhost:8080"),
        "ws://localhost:8080/maker"
    );
}

#[test]
fn normalizes_http_scheme() {
    assert_eq!(
        normalize_maker_ws_url("http://host:8080"),
        "ws://host:8080/maker"
    );
}

#[test]
fn normalizes_https_scheme() {
    assert_eq!(
        normalize_maker_ws_url("https://host:443"),
        "wss://host:443/maker"
    );
}

#[test]
fn leaves_full_maker_url_unchanged() {
    assert_eq!(
        normalize_maker_ws_url("wss://host/maker"),
        "wss://host/maker"
    );
}

#[test]
fn strips_trailing_slash_before_check() {
    assert_eq!(
        normalize_maker_ws_url("ws://localhost:8080/"),
        "ws://localhost:8080/maker"
    );
}

#[test]
fn session_error_delivered_to_single_awaiter() {
    let mut tracker = AwaitTracker::new();
    let (tx, mut rx) = oneshot::channel();
    tracker.register("Subscriptions", Some(Uuid::new_v4()), tx);

    let error_msg = ServerMessage::Error(ServerError::InternalError);
    let sender = tracker.take_for_message(&error_msg);
    assert!(sender.is_some(), "should route to the single awaiter");

    // Verify the awaiter's channel is now resolved.
    let sender = sender.unwrap();
    let _ = sender.send(Ok(Arc::new(error_msg)));
    let result = rx.try_recv();
    assert!(result.is_ok());
}

#[test]
fn session_error_not_delivered_with_multiple_awaiters() {
    let mut tracker = AwaitTracker::new();
    let (tx1, _rx1) = oneshot::channel();
    let (tx2, _rx2) = oneshot::channel();
    tracker.register("Subscriptions", Some(Uuid::new_v4()), tx1);
    tracker.register("MyQuotes", Some(Uuid::new_v4()), tx2);

    let error_msg = ServerMessage::Error(ServerError::InternalError);
    let sender = tracker.take_for_message(&error_msg);
    assert!(sender.is_none(), "should not route with ambiguous awaiters");
}

#[test]
fn request_error_routed_by_request_id() {
    let mut tracker = AwaitTracker::new();
    let id = Uuid::new_v4();
    let (tx, _rx) = oneshot::channel();
    tracker.register("SubscribeAck", Some(id), tx);

    let msg = ServerMessage::RequestError(RequestErrorEnvelope {
        request_id: id,
        error: ServerError::InternalError,
    });
    let sender = tracker.take_for_message(&msg);
    assert!(sender.is_some(), "should match by request_id");
}

#[test]
fn request_error_not_routed_on_id_mismatch() {
    let mut tracker = AwaitTracker::new();
    let (tx, _rx) = oneshot::channel();
    tracker.register("SubscribeAck", Some(Uuid::new_v4()), tx);

    let msg = ServerMessage::RequestError(RequestErrorEnvelope {
        request_id: Uuid::new_v4(),
        error: ServerError::InternalError,
    });
    let sender = tracker.take_for_message(&msg);
    assert!(sender.is_none(), "different request_id should not match");
}

#[test]
fn variant_matched_by_name_and_request_id() {
    let mut tracker = AwaitTracker::new();
    let id = Uuid::new_v4();
    let (tx, _rx) = oneshot::channel();
    tracker.register("SubscribeAck", Some(id), tx);

    let msg = ServerMessage::SubscribeAck(SubscribeAckData {
        request_id: id,
        subscribed: vec![WsChannel::Rfqs],
    });
    let sender = tracker.take_for_message(&msg);
    assert!(sender.is_some(), "should match variant + request_id");
}

#[test]
fn variant_matched_without_request_id_when_unambiguous() {
    let mut tracker = AwaitTracker::new();
    let (tx, _rx) = oneshot::channel();
    tracker.register("SubscribeAck", None, tx);

    let id = Uuid::new_v4();
    let msg = ServerMessage::SubscribeAck(SubscribeAckData {
        request_id: id,
        subscribed: vec![],
    });
    let sender = tracker.take_for_message(&msg);
    assert!(
        sender.is_some(),
        "single awaiter with no request_id should match"
    );
}

#[test]
fn variant_not_matched_when_multiple_awaiters_same_type() {
    let mut tracker = AwaitTracker::new();
    let (tx1, _rx1) = oneshot::channel();
    let (tx2, _rx2) = oneshot::channel();
    tracker.register("SubscribeAck", None, tx1);
    tracker.register("SubscribeAck", None, tx2);

    let msg = ServerMessage::SubscribeAck(SubscribeAckData {
        request_id: Uuid::new_v4(),
        subscribed: vec![],
    });
    let sender = tracker.take_for_message(&msg);
    assert!(sender.is_none(), "ambiguous awaiters should not match");
}

#[test]
fn remove_latest_returns_last_registered() {
    let mut tracker = AwaitTracker::new();
    let (tx1, _rx1) = oneshot::channel();
    let (tx2, mut rx2) = oneshot::channel();
    tracker.register("SubscribeAck", Some(Uuid::new_v4()), tx1);
    tracker.register("SubscribeAck", Some(Uuid::new_v4()), tx2);

    let sender = tracker.remove_latest();
    assert!(sender.is_some());
    let sender = sender.unwrap();
    let _ = sender.send(Ok(Arc::new(ServerMessage::Error(
        ServerError::InternalError,
    ))));
    assert!(
        rx2.try_recv().is_ok(),
        "should be the second (latest) registration"
    );
}

#[test]
fn remove_latest_returns_none_when_empty() {
    let mut tracker = AwaitTracker::new();
    assert!(tracker.remove_latest().is_none());
}

#[test]
fn drain_all_sends_disconnected_to_all() {
    let mut tracker = AwaitTracker::new();
    let (tx1, mut rx1) = oneshot::channel();
    let (tx2, mut rx2) = oneshot::channel();
    tracker.register("SubscribeAck", Some(Uuid::new_v4()), tx1);
    tracker.register("SubscribeAck", Some(Uuid::new_v4()), tx2);

    tracker.drain_all();

    let r1 = rx1.try_recv().unwrap();
    let r2 = rx2.try_recv().unwrap();
    assert!(matches!(r1, Err(SendAwaitError::Disconnected)));
    assert!(matches!(r2, Err(SendAwaitError::Disconnected)));
}

#[test]
fn take_for_message_returns_none_when_empty() {
    let mut tracker = AwaitTracker::new();
    let msg = ServerMessage::Error(ServerError::InternalError);
    assert!(tracker.take_for_message(&msg).is_none());
}
