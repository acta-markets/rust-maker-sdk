use std::sync::Arc;

use super::{
    GapPolicy, MakerWsEndpoint, ManagedCommand, ManagedReceiveError, ManagedWsConfig,
    ManagedWsError, ManagedWsEvent, ManagedWsState, ManagedWsTerminationReason,
    OutboundMessageError, SendAwaitError, WaitUntilReadyError, normalize_maker_data_ws_url,
    normalize_maker_ws_url, normalize_maker_ws_url_for_endpoint, spawn_managed_ws,
    tracker::AwaitTracker, transition_state,
};
use crate::types::RfqCloseReason;
use crate::types::ids::{Nonce, OrderId, Price, Strike};
use crate::ws::types::common::WsChannel;
use crate::ws::types::{
    AuthRequestData, AuthSuccessData, BatchQuoteResult, BatchQuotesAckMessage, BatchQuotesMessage,
    CancelQuoteData, CancelRfqData, ClientMessage, GetMmSummaryMessage, QuoteAcknowledgedMessage,
    QuoteMessage, QuoteRejectReason, QuoteRejectedMessage, RequestErrorEnvelope, RfqClosedMessage,
    ServerError, ServerMessage, SubscribeData, WelcomeData,
};
use futures_util::{SinkExt, StreamExt};
use std::time::{Duration, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
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
        Arc::new(crate::orders::BytesSigner::from_secret([1u8; 32])),
    )
    .with_auth_pubkey("maker")
}

#[test]
fn quote_defaults_enable_disconnect_cancellation_and_gap_recovery() {
    let config = managed_config();
    assert!(config.cancel_on_disconnect);
    assert!(config.reconnect_on_gap());
    assert!(
        config
            .hello()
            .features
            .iter()
            .any(|f| f == "cancel_on_disconnect")
    );
    let data = config.with_endpoint(MakerWsEndpoint::Data);
    assert!(!data.reconnect_on_gap());
    assert!(
        data.clone()
            .with_gap_policy(GapPolicy::Reconnect)
            .reconnect_on_gap()
    );
    assert!(data.clone().low_latency().reconnect_on_gap());
    assert!(
        !data
            .hello()
            .features
            .iter()
            .any(|f| f == "cancel_on_disconnect")
    );
}

#[test]
fn auth_pubkey_override_wins_over_the_signer_key() {
    use crate::orders::SignerLike;

    struct Remote(crate::orders::BytesSigner);
    impl crate::signing::AsyncSignerLike for Remote {
        fn pubkey_bytes(&self) -> [u8; 32] {
            self.0.pubkey_bytes()
        }
        fn sign_message<'a>(&'a self, message: &'a [u8]) -> crate::signing::SigningFuture<'a> {
            Box::pin(async move { Ok(self.0.sign_message(message)) })
        }
    }

    let signer = crate::orders::BytesSigner::from_secret([3u8; 32]);
    let expected = signer.pubkey_base58();
    let config = ManagedWsConfig::new_async(
        "ws://localhost",
        crate::ws::types::HelloData {
            protocol_version: "1".to_string(),
            features: Vec::new(),
            client_name: None,
            client_version: None,
        },
        Arc::new(Remote(signer)),
    );
    assert_eq!(config.auth_pubkey, expected);
    assert_eq!(config.with_auth_pubkey("owner").auth_pubkey, "owner");
}

#[test]
fn data_endpoint_rejects_initial_subscription_in_any_builder_order() {
    let subscribe = || SubscribeData {
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: None,
        quote_mints: None,
    };

    for config in [
        managed_config()
            .with_initial_subscribe(subscribe())
            .with_endpoint(MakerWsEndpoint::Data),
        managed_config()
            .with_endpoint(MakerWsEndpoint::Data)
            .with_initial_subscribe(subscribe()),
    ] {
        assert_eq!(
            config.validate(),
            Err(super::ManagedWsConfigError::InitialSubscribeOnDataEndpoint)
        );
    }
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
fn config_rejects_invalid_reconnect_jitter() {
    let mut config = managed_config();
    config.reconnect_jitter_ratio = 1.01;
    assert_eq!(
        config.validate(),
        Err(super::ManagedWsConfigError::InvalidReconnectJitter)
    );
}

#[test]
fn config_rejects_values_above_server_limits() {
    let mut config = managed_config();
    config.max_batch_quotes = 51;
    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::ProtocolLimitExceeded {
            field: "max_batch_quotes",
            configured: 51,
            limit: 50,
        })
    ));

    let mut config = managed_config();
    config.max_outbound_message_size = 32 * 1024 + 1;
    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::ProtocolLimitExceeded {
            field: "max_outbound_message_size",
            configured,
            limit: 32_768,
        }) if configured == 32 * 1024 + 1
    ));
}

#[test]
fn config_rejects_oversized_initial_subscription() {
    let mut config = managed_config().with_initial_subscribe(SubscribeData {
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: Some(vec!["x".repeat(256)]),
        quote_mints: None,
    });
    config.max_outbound_message_size = 128;

    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::InitialSubscribeTooLarge {
            actual,
            limit: 128,
        }) if actual > 128
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
fn managed_connect_timeout_is_the_single_effective_deadline() {
    let mut config = managed_config();
    config.connect_timeout = Duration::from_secs(3);
    config.transport.connect_timeout = Duration::ZERO;

    assert_eq!(
        config.effective_transport().connect_timeout,
        Duration::from_secs(3)
    );
    assert!(config.validate().is_ok());
}

#[test]
fn config_rejects_multi_gigabyte_outbound_envelopes() {
    let mut config = managed_config();
    config.command_buffer = 1024 * 1024;

    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::MemoryEnvelopeTooLarge {
            queue: "outbound command queue",
            ..
        })
    ));
}

#[test]
fn inbound_ring_can_be_raised_above_the_default() {
    let mut config = managed_config();
    config.transport.max_message_size = 2 * 1024 * 1024;

    for capacity in [64, 65, 1024, 65_536] {
        config.broadcast_buffer = capacity;
        assert!(
            config.validate().is_ok(),
            "broadcast_buffer {capacity} should be accepted"
        );
    }
}

#[test]
fn an_implausible_inbound_ring_is_still_rejected() {
    let mut config = managed_config();
    config.broadcast_buffer = 65_537;

    assert!(matches!(
        config.validate(),
        Err(super::ManagedWsConfigError::CapacityTooLarge {
            field: "broadcast_buffer",
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
        ManagedCommand::Send { message, tx, .. } => {
            assert_eq!(message.as_str(), r#"{"type":"Ping"}"#);
            tx.send(Ok(())).unwrap();
        }
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

#[tokio::test]
async fn first_subscriber_receives_bounded_since_spawn_backlog() {
    let (handle, _commands) = super::ManagedWsHandle::test_handle(1, 2);
    handle.inject_message(ServerMessage::Pong(crate::ws::types::PongData {
        server_time_unix_ms: UNIX_EPOCH,
    }));
    handle.inject_event(ManagedWsEvent::Connected);

    let message = handle.subscribe_messages().recv().await.unwrap();
    assert_eq!(message.sequence, 1);
    assert!(matches!(
        handle.subscribe_events().recv().await,
        Ok(ManagedWsEvent::Connected)
    ));
}

#[tokio::test]
async fn later_subscribers_start_at_subscription_time() {
    let (handle, _commands) = super::ManagedWsHandle::test_handle(1, 2);
    let _first = handle.subscribe_messages();
    handle.inject_message(ServerMessage::Pong(crate::ws::types::PongData {
        server_time_unix_ms: UNIX_EPOCH,
    }));
    let mut later = handle.subscribe_messages();
    handle.inject_message(ServerMessage::Pong(crate::ws::types::PongData {
        server_time_unix_ms: UNIX_EPOCH,
    }));

    assert_eq!(later.recv().await.unwrap().sequence, 2);
}

#[tokio::test]
async fn readiness_is_latest_state_and_terminal_failure_is_typed() {
    let (handle, _commands) = super::ManagedWsHandle::test_handle(1, 1);
    assert!(matches!(
        handle.ensure_ready(),
        Err(ManagedWsError::NotReady)
    ));

    handle.inject_state(ManagedWsState::Ready {
        connection_epoch: 7,
    });
    assert_eq!(handle.wait_until_ready().await.unwrap(), 7);
    assert!(handle.ensure_ready().is_ok());

    handle.inject_state(ManagedWsState::Closed {
        reason: ManagedWsTerminationReason::ReconnectLimitReached { limit: 3 },
    });
    assert!(matches!(
        handle.wait_until_ready().await,
        Err(WaitUntilReadyError::Terminated(
            ManagedWsTerminationReason::ReconnectLimitReached { limit: 3 }
        ))
    ));
}

#[tokio::test]
async fn lifecycle_event_observers_see_the_matching_latest_state() {
    let (state_tx, state_rx) = watch::channel(ManagedWsState::Connecting);
    let (events_tx, mut events_rx) = broadcast::channel(1);

    transition_state(
        &state_tx,
        &events_tx,
        ManagedWsState::Ready {
            connection_epoch: 7,
        },
        ManagedWsEvent::Ready,
    );

    assert!(matches!(events_rx.recv().await, Ok(ManagedWsEvent::Ready)));
    assert!(matches!(
        *state_rx.borrow(),
        ManagedWsState::Ready {
            connection_epoch: 7
        }
    ));
}

#[tokio::test]
async fn concurrent_close_waits_for_the_session_task() {
    let (handle, _commands) = super::ManagedWsHandle::make_test_handle(1, 1);
    let (finish_tx, finish_rx) = oneshot::channel();
    *handle.task.lock().await = Some(tokio::spawn(async move {
        finish_rx.await.unwrap();
    }));
    let other = handle.clone();
    let first = handle.close();
    let second = other.close();
    tokio::pin!(first, second);

    assert!(futures_util::poll!(&mut first).is_pending());
    assert!(futures_util::poll!(&mut second).is_pending());
    finish_tx.send(()).unwrap();
    first.await.unwrap();
    second.await.unwrap();
}

#[tokio::test]
async fn cancelled_close_preserves_the_session_join_for_the_next_caller() {
    let (handle, _commands) = super::ManagedWsHandle::make_test_handle(1, 1);
    let (finish_tx, finish_rx) = oneshot::channel();
    *handle.task.lock().await = Some(tokio::spawn(async move {
        finish_rx.await.unwrap();
    }));
    {
        let first = handle.close();
        tokio::pin!(first);
        assert!(futures_util::poll!(&mut first).is_pending());
    }
    let second = handle.close();
    tokio::pin!(second);
    assert!(futures_util::poll!(&mut second).is_pending());
    finish_tx.send(()).unwrap();
    second.await.unwrap();
}

#[tokio::test]
async fn close_interrupts_a_stalled_websocket_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled websocket peer");
    let address = listener.local_addr().expect("stalled peer address");
    let (accepted_tx, accepted_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.expect("accept stalled connection");
        let _ = accepted_tx.send(());
        std::future::pending::<()>().await;
    });

    let mut config = managed_config();
    config.url = format!("ws://{address}");
    config.connect_timeout = Duration::from_secs(30);
    let handle = spawn_managed_ws(config).expect("spawn managed websocket");
    tokio::time::timeout(Duration::from_secs(1), accepted_rx)
        .await
        .expect("managed client should reach peer")
        .expect("peer should report accepted connection");

    tokio::time::timeout(Duration::from_secs(1), handle.close())
        .await
        .expect("close must interrupt connect immediately")
        .expect("managed websocket should close cleanly");
    assert!(matches!(
        handle.state(),
        ManagedWsState::Closed {
            reason: ManagedWsTerminationReason::Requested
        }
    ));
    peer.abort();
}

#[tokio::test]
async fn close_interrupts_a_stalled_authentication() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled auth peer");
    let address = listener.local_addr().expect("stalled auth peer address");
    let (accepted_tx, accepted_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept auth connection");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        // Consume Hello but never answer, so authentication cannot progress.
        let _ = socket.next().await;
        let _ = accepted_tx.send(());
        std::future::pending::<()>().await;
    });

    let mut config = managed_config();
    config.url = format!("ws://{address}");
    config.auth_timeout = Duration::from_secs(30);
    let handle = spawn_managed_ws(config).expect("spawn managed websocket");
    tokio::time::timeout(Duration::from_secs(1), accepted_rx)
        .await
        .expect("managed client should send Hello")
        .expect("peer should report received Hello");

    tokio::time::timeout(Duration::from_secs(1), handle.close())
        .await
        .expect("close must interrupt authentication immediately")
        .expect("managed websocket should close cleanly");
    assert!(matches!(
        handle.state(),
        ManagedWsState::Closed {
            reason: ManagedWsTerminationReason::Requested
        }
    ));
    peer.abort();
}

#[tokio::test]
async fn close_interrupts_a_stalled_readiness_barrier() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled readiness peer");
    let address = listener
        .local_addr()
        .expect("stalled readiness peer address");
    let (authenticated_tx, authenticated_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept readiness connection");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        authenticate_test_peer(&mut socket).await;
        // Authentication succeeded; never acknowledge the initial subscribe.
        let _ = socket.next().await;
        let _ = authenticated_tx.send(());
        std::future::pending::<()>().await;
    });

    let mut config = managed_config().with_initial_subscribe(SubscribeData {
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: None,
        quote_mints: None,
    });
    config.url = format!("ws://{address}");
    config.readiness_timeout = Duration::from_secs(30);
    let handle = spawn_managed_ws(config).expect("spawn managed websocket");
    tokio::time::timeout(Duration::from_secs(2), authenticated_rx)
        .await
        .expect("managed client should reach the readiness barrier")
        .expect("peer should report the subscribe");

    tokio::time::timeout(Duration::from_secs(1), handle.close())
        .await
        .expect("close must interrupt the readiness barrier immediately")
        .expect("managed websocket should close cleanly");
    assert!(matches!(
        handle.state(),
        ManagedWsState::Closed {
            reason: ManagedWsTerminationReason::Requested
        }
    ));
    peer.abort();
}

#[tokio::test]
async fn dropping_the_last_handle_interrupts_a_stalled_websocket_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled websocket peer");
    let address = listener.local_addr().expect("stalled peer address");
    let (accepted_tx, accepted_rx) = oneshot::channel();
    let (closed_tx, closed_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept stalled connection");
        let _ = accepted_tx.send(());
        let mut buffer = [0_u8; 1024];
        while stream.read(&mut buffer).await.expect("read client socket") != 0 {}
        let _ = closed_tx.send(());
    });

    let mut config = managed_config();
    config.url = format!("ws://{address}");
    config.connect_timeout = Duration::from_secs(30);
    let handle = spawn_managed_ws(config).expect("spawn managed websocket");
    tokio::time::timeout(Duration::from_secs(1), accepted_rx)
        .await
        .expect("managed client should reach peer")
        .expect("peer should report accepted connection");

    drop(handle);

    tokio::time::timeout(Duration::from_secs(1), closed_rx)
        .await
        .expect("dropping the last handle must close the stalled connection")
        .expect("peer should observe socket closure");
    peer.await.expect("join stalled websocket peer");
}

async fn receive_client_message(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> ClientMessage {
    let message = socket
        .next()
        .await
        .expect("client message")
        .expect("valid websocket message");
    let Message::Text(text) = message else {
        panic!("expected client text message");
    };
    serde_json::from_str(&text).expect("valid client protocol message")
}

async fn send_server_message(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    message: ServerMessage,
) {
    socket
        .send(Message::Text(
            serde_json::to_string(&message)
                .expect("serialize server message")
                .into(),
        ))
        .await
        .expect("send server message");
}

async fn authenticate_test_peer(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) {
    assert!(matches!(
        receive_client_message(socket).await,
        ClientMessage::Hello(_)
    ));
    send_server_message(
        socket,
        ServerMessage::Welcome(WelcomeData {
            protocol_version: "1".to_string(),
            server_version: "test".to_string(),
            min_supported_version: "1".to_string(),
            enabled_features: vec!["cancel_on_disconnect".to_string()],
            server_time_unix_ms: std::time::UNIX_EPOCH,
        }),
    )
    .await;
    assert!(matches!(
        receive_client_message(socket).await,
        ClientMessage::StartAuth(_)
    ));
    send_server_message(
        socket,
        ServerMessage::AuthRequest(AuthRequestData {
            challenge: "challenge".to_string(),
        }),
    )
    .await;
    assert!(matches!(
        receive_client_message(socket).await,
        ClientMessage::AuthChallenge(_)
    ));
    send_server_message(
        socket,
        ServerMessage::AuthSuccess(AuthSuccessData {
            session_id: "session".to_string(),
            expires_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_900_000_000),
            maker_pda: None,
        }),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sustained_outbound_commands_do_not_starve_inbound_messages() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind saturated websocket peer");
    let address = listener.local_addr().expect("saturated peer address");
    let (send_pong_tx, send_pong_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept managed connection");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        authenticate_test_peer(&mut socket).await;
        recover_test_peer(&mut socket, false).await;
        send_pong_rx.await.expect("request server pong");
        send_server_message(
            &mut socket,
            ServerMessage::Pong(crate::ws::types::PongData {
                server_time_unix_ms: UNIX_EPOCH,
            }),
        )
        .await;
        while let Some(message) = socket.next().await {
            if message.is_err() {
                break;
            }
        }
    });

    let mut config = managed_config();
    config.url = format!("ws://{address}");
    config.command_buffer = 256;
    let handle = spawn_managed_ws(config).expect("spawn managed websocket");
    let mut messages = handle.subscribe_messages();
    handle.wait_until_ready().await.expect("ready session");

    let (stop_tx, stop_rx) = watch::channel(false);
    let mut producers = Vec::new();
    for _ in 0..64 {
        let producer = handle.clone();
        let stop_rx = stop_rx.clone();
        producers.push(tokio::spawn(async move {
            while !*stop_rx.borrow() {
                match producer.send(ClientMessage::Ping).await {
                    Ok(()) => {}
                    Err(ManagedWsError::Closed | ManagedWsError::Disconnected) => break,
                    Err(error) => panic!("unexpected saturated-send error: {error}"),
                }
            }
        }));
    }

    tokio::task::yield_now().await;
    send_pong_tx.send(()).expect("ask peer to send pong");
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let inbound = messages.recv().await.expect("receive managed message");
            if matches!(inbound.message(), ServerMessage::Pong(_)) {
                break;
            }
        }
    })
    .await
    .expect("inbound pong must not be starved by outbound commands");

    stop_tx.send_replace(true);
    for producer in producers {
        producer.await.expect("join saturated sender");
    }
    handle.close().await.expect("close managed websocket");
    peer.await.expect("join saturated websocket peer");
}

#[test]
fn low_latency_profile_validates_and_reconnects_on_gap() {
    let config = managed_config().low_latency();
    config
        .validate()
        .expect("low-latency profile is within limits");
    assert!(config.reconnect_on_gap());
    assert_eq!(config.ping_interval, Duration::from_secs(2));
    assert_eq!(config.pong_timeout, Duration::from_secs(2));
}

#[tokio::test]
async fn gap_policy_reconnect_reconnects_after_a_subscriber_lags() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gap peer");
    let address = listener.local_addr().expect("gap peer address");
    let (reconnected_tx, reconnected_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept first connection");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        authenticate_test_peer(&mut socket).await;
        recover_test_peer(&mut socket, true).await;
        let (stream, _) = listener.accept().await.expect("accept the reconnect");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        assert!(matches!(
            receive_client_message(&mut socket).await,
            ClientMessage::Hello(_)
        ));
        send_server_message(
            &mut socket,
            ServerMessage::Welcome(WelcomeData {
                protocol_version: "1".to_string(),
                server_version: "test".to_string(),
                min_supported_version: "1".to_string(),
                enabled_features: vec!["cancel_on_disconnect".to_string()],
                server_time_unix_ms: std::time::UNIX_EPOCH,
            }),
        )
        .await;
        assert!(matches!(
            receive_client_message(&mut socket).await,
            ClientMessage::ResumeAuth(_)
        ));
        send_server_message(
            &mut socket,
            ServerMessage::AuthSuccess(AuthSuccessData {
                session_id: "session".to_string(),
                expires_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_900_000_000),
                maker_pda: None,
            }),
        )
        .await;
        recover_test_peer(&mut socket, false).await;
        let _ = reconnected_tx.send(());
        while let Some(message) = socket.next().await {
            if message.is_err() {
                break;
            }
        }
    });

    let mut config = managed_config().with_initial_subscribe(SubscribeData {
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: None,
        quote_mints: None,
    });
    config.url = format!("ws://{address}");
    config.broadcast_buffer = 4;
    config.gap_policy = GapPolicy::Reconnect;
    let handle = spawn_managed_ws(config).expect("spawn managed websocket");
    let mut messages = handle.subscribe_messages();
    let mut state = handle.subscribe_state();
    handle.wait_until_ready().await.expect("ready session");

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match messages.recv().await {
                Err(ManagedReceiveError::Gap { .. }) => break,
                Ok(_) => {}
                Err(error) => panic!("unexpected receive error: {error}"),
            }
        }
    })
    .await
    .expect("the lagging subscriber sees a gap");

    tokio::time::timeout(Duration::from_secs(2), reconnected_rx)
        .await
        .expect("session reconnects after the gap")
        .expect("peer reports the reconnect");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                *state.borrow(),
                ManagedWsState::Ready {
                    connection_epoch: 2
                }
            ) {
                break;
            }
            state.changed().await.expect("state channel");
        }
    })
    .await
    .expect("ready again on the next epoch");

    handle.close().await.expect("close managed websocket");
    peer.await.expect("join gap peer");
}

#[tokio::test]
async fn backend_session_replaced_frame_terminates_without_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind control-frame peer");
    let address = listener.local_addr().expect("control-frame peer address");
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept managed connection");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        let _hello = socket
            .next()
            .await
            .expect("hello frame")
            .expect("valid hello frame");
        let frame = ServerMessage::Error(ServerError::Generic {
            code: "session_replaced".to_string(),
            message: "This session was replaced by a newer connection".to_string(),
        });
        socket
            .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
            .await
            .expect("send terminal control frame");
    });

    let mut config = managed_config();
    config.url = format!("ws://{address}");
    let handle = spawn_managed_ws(config).expect("spawn managed websocket");

    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), handle.wait_until_ready()).await,
        Ok(Err(WaitUntilReadyError::Terminated(
            ManagedWsTerminationReason::SessionReplaced
        )))
    ));
    assert!(matches!(
        handle.state(),
        ManagedWsState::Closed {
            reason: ManagedWsTerminationReason::SessionReplaced
        }
    ));
    handle.close().await.expect("join managed task");
    peer.await.expect("join control-frame peer");
}

fn quote(order_byte: u8) -> QuoteMessage {
    QuoteMessage {
        rfq_id: Uuid::new_v4(),
        strike: Strike::new(1),
        price: Price::new(2),
        valid_until: crate::QuoteExpiry::from_unix_seconds(100),
        nonce: Nonce::new(3),
        order_id: OrderId::new([order_byte; 32]),
        signature: "signature".to_string(),
    }
}

fn mm_summary() -> ClientMessage {
    ClientMessage::GetMmSummary(GetMmSummaryMessage {
        request_id: Uuid::new_v4(),
    })
}

fn register(
    tracker: &mut AwaitTracker,
    await_id: u64,
    message: &ClientMessage,
) -> oneshot::Receiver<Result<Arc<ServerMessage>, SendAwaitError>> {
    let (tx, rx) = oneshot::channel();
    tracker
        .register(await_id, AwaitTracker::prepare(message).unwrap(), tx)
        .unwrap();
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
fn cancel_rfq_without_a_success_receipt_is_not_awaitable() {
    let rfq_id = Uuid::new_v4();
    let message = ClientMessage::CancelRfq(CancelRfqData {
        rfq_id,
        request_id: Uuid::new_v4(),
    });
    assert!(matches!(
        AwaitTracker::prepare(&message),
        Err(SendAwaitError::NoCorrelationKey)
    ));
    let mut tracker = AwaitTracker::new(2);
    let _rx = register(
        &mut tracker,
        1,
        &ClientMessage::CancelQuote(CancelQuoteData {
            rfq_id,
            request_id: Uuid::new_v4(),
        }),
    );
    for reason in [
        RfqCloseReason::TakerCancelled,
        RfqCloseReason::Expired,
        RfqCloseReason::Filled,
    ] {
        let closed = ServerMessage::RfqClosed(RfqClosedMessage {
            rfq_id,
            rfq_version: Default::default(),
            reason,
            your_quote: None,
            winner: None,
            closed_at: UNIX_EPOCH,
        });
        assert!(tracker.take_for_message(&closed).is_none());
    }
    assert_eq!(tracker.len(), 1);
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
fn batch_ack_with_future_status_still_correlates_by_order_id() {
    let mut tracker = AwaitTracker::new(2);
    let first = quote(14);
    let second = quote(15);
    let batch = ClientMessage::BatchQuotes(BatchQuotesMessage {
        quotes: vec![first.clone(), second.clone()],
    });
    let _rx = register(&mut tracker, 1, &batch);
    let ack = ServerMessage::BatchQuotesAck(BatchQuotesAckMessage {
        results: vec![
            BatchQuoteResult::Acknowledged(QuoteAcknowledgedMessage {
                rfq_id: first.rfq_id,
                order_id: first.order_id,
                replaced_order_id: None,
            }),
            BatchQuoteResult::Unknown(crate::ws::types::UnknownBatchQuoteResult {
                status: "deferred".to_string(),
                data: Some(serde_json::json!({ "order_id": second.order_id })),
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

    let error = tracker
        .register(2, AwaitTracker::prepare(&message).unwrap(), tx)
        .unwrap_err()
        .0;
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
fn externally_cancelled_waiter_does_not_pin_its_correlation_key() {
    let mut tracker = AwaitTracker::new(1);
    let message = ClientMessage::Quote(quote(16));
    let receiver = register(&mut tracker, 21, &message);
    drop(receiver);

    let _receiver = register(&mut tracker, 22, &message);

    assert_eq!(tracker.len(), 1);
}

#[test]
fn tracker_enforces_capacity() {
    let mut tracker = AwaitTracker::new(1);
    let _rx = register(&mut tracker, 1, &ClientMessage::Quote(quote(8)));
    let (tx, _rx) = oneshot::channel();
    let error = tracker
        .register(
            2,
            AwaitTracker::prepare(&ClientMessage::Quote(quote(9))).unwrap(),
            tx,
        )
        .unwrap_err()
        .0;
    assert!(matches!(error, SendAwaitError::TooManyPending { limit: 1 }));
}

#[tokio::test]
async fn managed_handle_rejects_oversized_quote_batch_before_queueing() {
    let (mut handle, mut commands) = super::ManagedWsHandle::test_handle(1, 1);
    handle.max_batch_quotes = 1;
    handle.inject_state(ManagedWsState::Ready {
        connection_epoch: 1,
    });
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

#[test]
fn cancel_ack_correlates_requests_and_ignores_lifecycle_events() {
    use crate::ws::types::{CancelQuoteAckMessage, QuoteCancelReason, QuoteCancelledMessage};
    let mut tracker = AwaitTracker::new(2);
    let rfq_id = Uuid::new_v4();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let _first_rx = register(
        &mut tracker,
        1,
        &ClientMessage::CancelQuote(CancelQuoteData {
            rfq_id,
            request_id: first,
        }),
    );
    let _second_rx = register(
        &mut tracker,
        2,
        &ClientMessage::CancelQuote(CancelQuoteData {
            rfq_id,
            request_id: second,
        }),
    );
    assert!(
        tracker
            .take_for_message(&ServerMessage::QuoteCancelled(QuoteCancelledMessage {
                rfq_id,
                order_ids: vec![],
                reason: QuoteCancelReason::Requested,
                cancelled_at: UNIX_EPOCH,
            }))
            .is_none()
    );
    let ack = |request_id| {
        ServerMessage::CancelQuoteAck(CancelQuoteAckMessage {
            request_id,
            rfq_id,
            cancelled_order_ids: vec![],
        })
    };
    assert!(tracker.take_for_message(&ack(second)).is_some());
    assert!(tracker.take_for_message(&ack(second)).is_none());
    assert_eq!(tracker.len(), 1);
    assert!(tracker.take_for_message(&ack(first)).is_some());
    assert_eq!(tracker.len(), 0);
}

#[tokio::test]
async fn raw_managed_handle_rejects_mutations_before_ready() {
    let (handle, _commands) = super::ManagedWsHandle::test_handle(1, 1);
    handle.inject_state(ManagedWsState::Authenticated {
        connection_epoch: 1,
    });
    let message = ClientMessage::Quote(quote(1));
    assert!(matches!(
        handle.send(message.clone()).await,
        Err(ManagedWsError::NotReady)
    ));
    assert!(matches!(
        handle.try_send(message.clone()),
        Err(ManagedWsError::NotReady)
    ));
    assert!(matches!(
        handle.send_await(message, Duration::from_secs(1)).await,
        Err(SendAwaitError::NotReady)
    ));
}

#[tokio::test]
async fn explicit_epoch_send_rejects_stale_or_unready_connection_without_enqueueing() {
    let (handle, mut commands) = super::ManagedWsHandle::test_handle(1, 1);
    for state in [
        ManagedWsState::Connecting,
        ManagedWsState::Authenticated {
            connection_epoch: 1,
        },
        ManagedWsState::Ready {
            connection_epoch: 2,
        },
    ] {
        handle.inject_state(state);
        assert!(matches!(
            handle
                .send_in_epoch(ClientMessage::Quote(quote(1)), 1)
                .await,
            Err(ManagedWsError::NotReady),
        ));
        assert!(matches!(
            commands.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn explicit_epoch_send_keeps_the_checked_epoch_and_returns_write_result() {
    let (handle, mut commands) = super::ManagedWsHandle::test_handle(1, 1);
    handle.inject_state(ManagedWsState::Ready {
        connection_epoch: 7,
    });
    let send = handle.send_in_epoch(ClientMessage::Quote(quote(1)), 7);
    tokio::pin!(send);
    assert!(futures_util::poll!(send.as_mut()).is_pending());
    match commands.recv().await.unwrap() {
        ManagedCommand::Send {
            connection_epoch,
            message,
            tx,
        } => {
            assert_eq!(connection_epoch, Some(7));
            assert!(matches!(
                serde_json::from_str::<ClientMessage>(message.as_str()).unwrap(),
                ClientMessage::Quote(_)
            ));
            tx.send(Ok(())).unwrap();
        }
        _ => panic!("expected Send"),
    }
    send.await.unwrap();
}

#[tokio::test]
async fn ready_queries_are_enqueued_with_the_connection_epoch() {
    let (handle, mut commands) = super::ManagedWsHandle::test_handle(2, 1);
    handle.inject_state(ManagedWsState::Ready {
        connection_epoch: 7,
    });

    let send_handle = handle.clone();
    let send = tokio::spawn(async move { send_handle.send(mm_summary()).await });
    match commands.recv().await.unwrap() {
        ManagedCommand::Send {
            connection_epoch,
            tx,
            ..
        } => {
            assert_eq!(connection_epoch, Some(7));
            tx.send(Ok(())).unwrap();
        }
        _ => panic!("expected a send command"),
    }
    assert!(send.await.unwrap().is_ok());

    let await_handle = handle.clone();
    let send_await = tokio::spawn(async move {
        await_handle
            .send_await(mm_summary(), Duration::from_secs(1))
            .await
    });
    match commands.recv().await.unwrap() {
        ManagedCommand::SendAwait {
            connection_epoch,
            tx,
            ..
        } => {
            assert_eq!(connection_epoch, Some(7));
            tx.send(Err(SendAwaitError::Disconnected)).unwrap();
        }
        _ => panic!("expected a send-await command"),
    }
    assert!(matches!(
        send_await.await.unwrap(),
        Err(SendAwaitError::Disconnected)
    ));
}

#[tokio::test]
async fn indicative_response_rechecks_readiness_before_enqueue() {
    let (handle, mut commands) = super::ManagedWsHandle::test_handle(2, 1);
    let response = ClientMessage::IndicativePricesResponse(
        crate::ws::types::IndicativePricesResponseMessage {
            request_id: Uuid::new_v4(),
            market: "market".into(),
            position_type: crate::PositionType::CoveredCall,
            prices: Vec::new(),
        },
    );
    handle.inject_state(ManagedWsState::Ready {
        connection_epoch: 1,
    });
    handle.ensure_ready().unwrap();
    handle.inject_state(ManagedWsState::Connecting);

    assert!(matches!(
        handle.try_send(response.clone()),
        Err(ManagedWsError::NotReady)
    ));
    let send = handle.send(response.clone());
    tokio::pin!(send);
    assert!(matches!(
        futures_util::poll!(&mut send),
        std::task::Poll::Ready(Err(ManagedWsError::NotReady))
    ));
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));

    handle.inject_state(ManagedWsState::Ready {
        connection_epoch: 2,
    });
    let send = handle.send(response);
    tokio::pin!(send);
    assert!(futures_util::poll!(&mut send).is_pending());
    match commands.recv().await.unwrap() {
        ManagedCommand::Send {
            connection_epoch,
            tx,
            ..
        } => {
            assert_eq!(connection_epoch, Some(2));
            tx.send(Ok(())).unwrap();
        }
        _ => panic!("expected Send"),
    }
    send.await.unwrap();
}

#[tokio::test]
async fn pre_ready_queries_remain_unfenced() {
    let (handle, mut commands) = super::ManagedWsHandle::test_handle(1, 1);
    handle.inject_state(ManagedWsState::Authenticated {
        connection_epoch: 7,
    });

    let send_handle = handle.clone();
    let send = tokio::spawn(async move { send_handle.send(mm_summary()).await });
    match commands.recv().await.unwrap() {
        ManagedCommand::Send {
            connection_epoch,
            tx,
            ..
        } => {
            assert_eq!(connection_epoch, None);
            tx.send(Ok(())).unwrap();
        }
        _ => panic!("expected a send command"),
    }
    assert!(send.await.unwrap().is_ok());
}

async fn recover_test_peer(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    overflow: bool,
) {
    use crate::ws::types::*;
    let mut reads = 0;
    while reads < 3 {
        let request = receive_client_message(socket).await;
        let reply = match request {
            ClientMessage::GetSubscriptions(m) => {
                ServerMessage::Subscriptions(SubscriptionsMessage {
                    request_id: m.request_id,
                    channels: vec![],
                    underlying_mints: None,
                    quote_mints: None,
                })
            }
            ClientMessage::Subscribe(m) => {
                if overflow {
                    for _ in 0..16 {
                        send_server_message(
                            socket,
                            ServerMessage::Pong(PongData {
                                server_time_unix_ms: UNIX_EPOCH,
                            }),
                        )
                        .await;
                    }
                }
                ServerMessage::SubscribeAck(SubscribeAckData {
                    request_id: m.request_id,
                    subscribed: m.channels,
                })
            }
            ClientMessage::GetMmSummary(m) => {
                reads += 1;
                ServerMessage::MmSummary(MmSummaryData {
                    request_id: m.request_id,
                    maker_pda: "maker".to_string(),
                    caps: MyCapsData {
                        request_id: m.request_id,
                        positions: MakerPositionCapInfo {
                            current: 0,
                            limit: 0,
                        },
                        notional: Vec::new(),
                        balances: Vec::new(),
                    },
                    positions: Vec::new(),
                    active_quotes: Vec::new(),
                    markets: Vec::new(),
                    tokens: Vec::new(),
                    computed_at: UNIX_EPOCH,
                    unrenderable_positions: Vec::new(),
                    positions_has_more: false,
                })
            }
            ClientMessage::GetActiveRfqs(m) => {
                reads += 1;
                ServerMessage::ActiveRfqs(ActiveRfqsData {
                    request_id: m.request_id,
                    rfqs: vec![],
                })
            }
            ClientMessage::GetMyQuotes(m) => {
                reads += 1;
                ServerMessage::MyQuotes(MyQuotesMessage {
                    request_id: m.request_id,
                    quotes: vec![],
                    has_more: false,
                })
            }
            other => panic!("unexpected recovery request {other:?}"),
        };
        send_server_message(socket, reply).await;
    }
}
