use std::sync::Arc;

use super::{
    MakerWsEndpoint, ManagedCommand, ManagedReceiveError, ManagedWsConfig, OutboundMessageError,
    SendAwaitError, normalize_maker_data_ws_url, normalize_maker_ws_url,
    normalize_maker_ws_url_for_endpoint, tracker::AwaitTracker,
};
use crate::types::RfqCloseReason;
use crate::types::ids::{Nonce, OrderId, Price, Strike};
use crate::ws::types::common::WsChannel;
use crate::ws::types::{
    BatchQuoteResult, BatchQuotesAckMessage, BatchQuotesMessage, CancelQuoteData, CancelRfqData,
    ClientMessage, QuoteAcknowledgedMessage, QuoteMessage, QuoteRejectReason, QuoteRejectedMessage,
    RequestErrorEnvelope, RfqClosedMessage, ServerError, ServerMessage, SubscribeData,
};
use std::time::{Duration, UNIX_EPOCH};
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
fn normalizes_maker_data_endpoint() {
    assert_eq!(
        normalize_maker_data_ws_url("https://host:443"),
        "wss://host:443/maker/data"
    );
    assert_eq!(
        normalize_maker_data_ws_url("wss://host/maker"),
        "wss://host/maker/data"
    );
    assert_eq!(
        normalize_maker_data_ws_url("wss://host/maker/data"),
        "wss://host/maker/data"
    );
}

#[test]
fn quote_endpoint_normalization_rewrites_maker_data_url() {
    assert_eq!(
        normalize_maker_ws_url("wss://host/maker/data"),
        "wss://host/maker"
    );
}

#[test]
fn endpoint_normalizer_accepts_explicit_endpoint() {
    assert_eq!(
        normalize_maker_ws_url_for_endpoint("ws://host", MakerWsEndpoint::Quote),
        "ws://host/maker"
    );
    assert_eq!(
        normalize_maker_ws_url_for_endpoint("ws://host", MakerWsEndpoint::Data),
        "ws://host/maker/data"
    );
}

#[test]
fn strips_trailing_slash_before_check() {
    assert_eq!(
        normalize_maker_ws_url("ws://localhost:8080/"),
        "ws://localhost:8080/maker"
    );
}

fn managed_config() -> ManagedWsConfig {
    ManagedWsConfig::new(
        "ws://localhost",
        crate::ws::types::HelloData {
            protocol_version: "1".to_string(),
            features: Vec::new(),
            client_name: None,
            client_version: None,
        },
        "maker",
        Arc::new(|_| Ok("signature".to_string())),
    )
}

#[test]
fn reconcile_defaults_to_data_plane_only() {
    assert!(!managed_config().auto_reconcile);
    assert!(
        managed_config()
            .with_endpoint(MakerWsEndpoint::Data)
            .auto_reconcile
    );
}

#[test]
fn config_rejects_zero_capacity_before_spawning() {
    let mut config = managed_config();
    config.command_buffer = 0;
    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::ZeroCapacity {
            field: "command_buffer"
        })
    ));
}

#[test]
fn config_rejects_a_zero_reconnect_loop() {
    let mut config = managed_config();
    config.reconnect_delay = std::time::Duration::ZERO;
    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::ZeroDuration {
            field: "reconnect_delay"
        })
    ));
}

#[test]
fn config_rejects_invalid_transport_limits_before_spawning() {
    let mut config = managed_config();
    config.transport.max_frame_size = 0;
    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::Transport(
            crate::ws::error::WsTransportConfigError::ZeroMessageOrFrameLimit
        ))
    ));
}

#[test]
fn config_rejects_multi_gigabyte_queue_envelopes() {
    let mut config = managed_config();
    config.broadcast_buffer = 1024;
    config.transport.max_message_size = 8 * 1024 * 1024;

    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::MemoryEnvelopeTooLarge {
            queue: "inbound broadcast ring",
            ..
        })
    ));
}

#[tokio::test]
async fn try_send_ticket_waits_for_socket_write_result() {
    let (handle, mut commands) = super::ManagedWsHandle::test_handle(1, 1);
    let ticket = handle.try_send(ClientMessage::Ping).unwrap();
    let command = commands.recv().await.unwrap();
    match command {
        ManagedCommand::Send { tx, .. } => tx.send(Ok(())).unwrap(),
        _ => panic!("expected Send"),
    }
    ticket.wait().await.unwrap();
}

#[tokio::test]
async fn slow_subscriber_gets_an_explicit_gap() {
    let (handle, _commands) = super::ManagedWsHandle::test_handle(1, 1);
    let mut messages = handle.subscribe_messages();
    handle.inject_message(ServerMessage::Pong(crate::ws::types::PongData {
        server_time_unix_ms: UNIX_EPOCH,
    }));
    handle.inject_message(ServerMessage::Pong(crate::ws::types::PongData {
        server_time_unix_ms: UNIX_EPOCH,
    }));

    assert!(matches!(
        messages.recv().await,
        Err(ManagedReceiveError::Gap { skipped: 1 })
    ));
    let next = messages.recv().await.unwrap();
    assert_eq!(next.sequence, 2);
}

fn quote(order_byte: u8) -> QuoteMessage {
    QuoteMessage {
        rfq_id: Uuid::new_v4(),
        strike: Strike::new(1),
        price: Price::new(2),
        valid_until: UNIX_EPOCH + Duration::from_secs(100),
        nonce: Nonce::new(3),
        order_id: OrderId::new([order_byte; 32]),
        signature: "signature".to_string(),
    }
}

fn register(
    tracker: &mut AwaitTracker,
    await_id: u64,
    message: &ClientMessage,
) -> oneshot::Receiver<Result<Arc<ServerMessage>, SendAwaitError>> {
    let (tx, rx) = oneshot::channel();
    tracker.register(await_id, message, tx).unwrap();
    rx
}

#[test]
fn request_error_is_routed_by_request_id() {
    let mut tracker = AwaitTracker::new(4);
    let request_id = Uuid::new_v4();
    let message = ClientMessage::Subscribe(SubscribeData {
        request_id,
        channels: vec![WsChannel::Rfqs],
        underlying_mints: None,
        quote_mints: None,
    });
    let _rx = register(&mut tracker, 1, &message);

    let error = ServerMessage::RequestError(RequestErrorEnvelope {
        request_id,
        error: ServerError::InternalError,
    });
    assert!(tracker.take_for_message(&error).is_some());
}

#[test]
fn cancel_quote_request_error_resolves_without_timeout() {
    let mut tracker = AwaitTracker::new(2);
    let request_id = Uuid::new_v4();
    let message = ClientMessage::CancelQuote(CancelQuoteData {
        rfq_id: Uuid::new_v4(),
        request_id,
    });
    let _rx = register(&mut tracker, 1, &message);

    let error = ServerMessage::RequestError(RequestErrorEnvelope {
        request_id,
        error: ServerError::InternalError,
    });
    assert!(tracker.take_for_message(&error).is_some());
    assert_eq!(tracker.len(), 0);
}

#[test]
fn cancel_rfq_success_resolves_by_rfq_id() {
    let mut tracker = AwaitTracker::new(2);
    let rfq_id = Uuid::new_v4();
    let message = ClientMessage::CancelRfq(CancelRfqData {
        rfq_id,
        request_id: Uuid::new_v4(),
    });
    let _rx = register(&mut tracker, 1, &message);

    let closed = ServerMessage::RfqClosed(RfqClosedMessage {
        rfq_id,
        rfq_version: Default::default(),
        reason: RfqCloseReason::TakerCancelled,
        your_quote: None,
        winner: None,
        closed_at: UNIX_EPOCH,
    });
    assert!(tracker.take_for_message(&closed).is_some());
    assert_eq!(tracker.len(), 0);
}

#[test]
fn concurrent_quote_acks_are_correlated_out_of_order() {
    let mut tracker = AwaitTracker::new(4);
    let first = quote(1);
    let second = quote(2);
    let mut first_rx = register(&mut tracker, 1, &ClientMessage::Quote(first.clone()));
    let mut second_rx = register(&mut tracker, 2, &ClientMessage::Quote(second.clone()));

    let second_ack = ServerMessage::QuoteAcknowledged(QuoteAcknowledgedMessage {
        rfq_id: second.rfq_id,
        order_id: second.order_id,
        replaced_order_id: None,
    });
    tracker
        .take_for_message(&second_ack)
        .unwrap()
        .send(Ok(Arc::new(second_ack)))
        .unwrap();

    assert!(second_rx.try_recv().is_ok());
    assert!(matches!(
        first_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
}

#[test]
fn quote_rejection_resolves_the_matching_quote() {
    let mut tracker = AwaitTracker::new(2);
    let quote = quote(3);
    let _rx = register(&mut tracker, 1, &ClientMessage::Quote(quote.clone()));
    let rejection = ServerMessage::QuoteRejected(QuoteRejectedMessage {
        rfq_id: quote.rfq_id,
        order_id: quote.order_id,
        reason: QuoteRejectReason::DuplicateOrderId,
        message: None,
    });

    assert!(tracker.take_for_message(&rejection).is_some());
}

#[test]
fn batch_ack_is_correlated_independently_of_result_order() {
    let mut tracker = AwaitTracker::new(2);
    let first = quote(4);
    let second = quote(5);
    let batch = ClientMessage::BatchQuotes(BatchQuotesMessage {
        quotes: vec![first.clone(), second.clone()],
    });
    let _rx = register(&mut tracker, 1, &batch);
    let ack = ServerMessage::BatchQuotesAck(BatchQuotesAckMessage {
        results: vec![
            BatchQuoteResult::Acknowledged(QuoteAcknowledgedMessage {
                rfq_id: second.rfq_id,
                order_id: second.order_id,
                replaced_order_id: None,
            }),
            BatchQuoteResult::Acknowledged(QuoteAcknowledgedMessage {
                rfq_id: first.rfq_id,
                order_id: first.order_id,
                replaced_order_id: None,
            }),
        ],
    });

    assert!(tracker.take_for_message(&ack).is_some());
}

#[test]
fn duplicate_in_flight_key_is_rejected() {
    let mut tracker = AwaitTracker::new(2);
    let message = ClientMessage::Quote(quote(6));
    let _rx = register(&mut tracker, 1, &message);
    let (tx, _rx) = oneshot::channel();

    let error = tracker.register(2, &message, tx).unwrap_err().0;
    assert!(matches!(error, SendAwaitError::DuplicateInFlight));
}

#[test]
fn cancellation_removes_timed_out_entry() {
    let mut tracker = AwaitTracker::new(1);
    let message = ClientMessage::Quote(quote(7));
    let _rx = register(&mut tracker, 11, &message);

    assert!(tracker.cancel(11).is_some());
    assert_eq!(tracker.len(), 0);
    let _rx = register(&mut tracker, 12, &message);
}

#[test]
fn tracker_enforces_capacity() {
    let mut tracker = AwaitTracker::new(1);
    let _rx = register(&mut tracker, 1, &ClientMessage::Quote(quote(8)));
    let (tx, _rx) = oneshot::channel();
    let error = tracker
        .register(2, &ClientMessage::Quote(quote(9)), tx)
        .unwrap_err()
        .0;
    assert!(matches!(error, SendAwaitError::TooManyPending { limit: 1 }));
}

#[tokio::test]
async fn managed_handle_rejects_oversized_quote_batch_before_queueing() {
    let (mut handle, mut commands) = super::ManagedWsHandle::test_handle(1, 1);
    handle.max_batch_quotes = 1;
    let message = ClientMessage::BatchQuotes(BatchQuotesMessage {
        quotes: vec![quote(1), quote(2)],
    });

    assert!(matches!(
        handle.send(message).await,
        Err(super::ManagedWsError::InvalidMessage(
            OutboundMessageError::BatchTooLarge {
                actual: 2,
                limit: 1
            }
        ))
    ));
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn managed_handle_rejects_oversized_serialized_message_before_queueing() {
    let (mut handle, mut commands) = super::ManagedWsHandle::test_handle(1, 1);
    handle.max_outbound_message_size = 1;

    assert!(matches!(
        handle.send(ClientMessage::Ping).await,
        Err(super::ManagedWsError::InvalidMessage(
            OutboundMessageError::MessageTooLarge { limit: 1, .. }
        ))
    ));
    assert!(commands.try_recv().is_err());
}

#[test]
fn session_error_is_never_guessed() {
    let mut tracker = AwaitTracker::new(1);
    let _rx = register(&mut tracker, 1, &ClientMessage::Quote(quote(10)));
    assert!(
        tracker
            .take_for_message(&ServerMessage::Error(ServerError::InternalError))
            .is_none()
    );
}

#[test]
fn drain_all_resolves_every_waiter_as_disconnected() {
    let mut tracker = AwaitTracker::new(2);
    let mut first = register(&mut tracker, 1, &ClientMessage::Quote(quote(11)));
    let mut second = register(&mut tracker, 2, &ClientMessage::Quote(quote(12)));

    tracker.drain_all();

    assert!(matches!(
        first.try_recv().unwrap(),
        Err(SendAwaitError::Disconnected)
    ));
    assert!(matches!(
        second.try_recv().unwrap(),
        Err(SendAwaitError::Disconnected)
    ));
}
