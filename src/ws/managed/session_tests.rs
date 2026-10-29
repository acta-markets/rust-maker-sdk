use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

use super::auth::AuthOutcome;
use super::{
    AuthenticatedSessionEnd, InboundPublisher, SessionEnd, SessionFailure, ready::establish,
};
use crate::orders::{BytesSigner, SignerLike};
use crate::ws::client::WsClient;
use crate::ws::managed::desired_subscriptions::DesiredSubscriptions;
use crate::ws::managed::{MakerWsEndpoint, ManagedWsConfig, ManagedWsTerminationReason};

struct ExplodingSigner;

impl SignerLike for ExplodingSigner {
    fn pubkey_bytes(&self) -> [u8; 32] {
        [7u8; 32]
    }

    fn sign_message(&self, _: &[u8]) -> [u8; 64] {
        panic!("signer exploded")
    }
}

impl crate::AsyncSignerLike for ExplodingSigner {
    fn pubkey_bytes(&self) -> [u8; 32] {
        [7; 32]
    }

    fn sign_message<'a>(&'a self, _: &'a [u8]) -> crate::signing::SigningFuture<'a> {
        panic!("async signer must not receive a rejected challenge")
    }
}

#[tokio::test]
async fn invalid_auth_never_reaches_either_signer() {
    let base = config("ws://localhost/maker".into());
    let sync = ManagedWsConfig::new(
        "ws://localhost/maker",
        base.hello(),
        Arc::new(ExplodingSigner),
        crate::Address32::new([135; 32]),
    );
    let asynchronous = ManagedWsConfig::new_async(
        "ws://localhost/maker",
        base.hello(),
        Arc::new(ExplodingSigner),
        crate::Address32::new([135; 32]),
    );
    for challenge in ["x".repeat(32), "é".repeat(16), "challenge".to_owned()] {
        for config in [&sync, &asynchronous] {
            let error = super::auth::build_auth_response(config, &challenge)
                .await
                .expect_err("reject before signing");
            assert_eq!(error.to_string(), "invalid maker authentication challenge");
        }
    }
}

#[tokio::test]
async fn canonical_auth_signs_original_bytes_for_delegated_owner() {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let signer = BytesSigner::from_secret([1; 32]);
    let owner = BytesSigner::from_secret([2; 32]).pubkey_base58();
    let config = ManagedWsConfig::new(
        "ws://localhost/maker",
        config("ws://localhost/maker".into()).hello(),
        Arc::new(signer.clone()),
        crate::Address32::from_base58(&owner.clone()).expect("valid fixture"),
    );
    let challenge = crate::ws::auth::TEST_MAKER_AUTH_CHALLENGE;
    let response = super::auth::build_auth_response(&config, challenge)
        .await
        .expect("canonical delegated auth");
    assert_eq!(
        response.pubkey,
        crate::Address32::from_base58(&owner).expect("valid fixture")
    );
    assert_ne!(
        response.pubkey,
        crate::Address32::new(signer.pubkey_bytes())
    );
    assert_eq!(response.challenge, challenge);
    let signature = crate::wire::decode_base58_64(&response.signature).expect("signature");
    VerifyingKey::from_bytes(&signer.pubkey_bytes())
        .expect("key")
        .verify(challenge.as_bytes(), &Signature::from_bytes(&signature))
        .expect("signature binds original auth bytes");
    assert_ne!(
        signature,
        crate::orders::sign_order_id_with_signer(&[0; 32], &signer)
    );
}
#[cfg(feature = "vault")]
#[tokio::test]
async fn vault_auth_binds_both_identities_before_either_signer() {
    let operator = crate::wire::encode_base58(&[7; 32]);
    let vault = crate::wire::encode_base58(&[8; 32]);
    let other = crate::wire::encode_base58(&[9; 32]);
    let base = config("ws://localhost/vault".into());
    let sync = ManagedWsConfig::new(
        "ws://localhost/vault",
        base.hello(),
        Arc::new(ExplodingSigner),
        crate::Address32::from_base58(&operator.clone()).expect("valid fixture"),
    )
    .with_vault_auth(
        crate::Address32::from_base58(&vault.clone()).expect("valid fixture"),
        crate::ProductSet::ALL,
    );
    let asynchronous = ManagedWsConfig::new_async(
        "ws://localhost/vault",
        base.hello(),
        Arc::new(ExplodingSigner),
        crate::Address32::from_base58(&operator.clone()).expect("valid fixture"),
    )
    .with_vault_auth(
        crate::Address32::from_base58(&vault.clone()).expect("valid fixture"),
        crate::ProductSet::ALL,
    );
    let canonical = crate::ws::auth::test_vault_auth_challenge(&operator, &vault);
    let invalid = [
        "x".repeat(32),
        crate::ws::auth::TEST_MAKER_AUTH_CHALLENGE.to_owned(),
        crate::ws::auth::test_vault_auth_challenge(&other, &vault),
        crate::ws::auth::test_vault_auth_challenge(&operator, &other),
        canonical.replace("Acta RFQ Authentication", "Acta Admin Authentication"),
        canonical.replace("2026-09-15", "2026-02-30"),
        canonical.replace("Nonce: ", "Nonce: a"),
        format!("{canonical}\n"),
    ];
    for challenge in invalid {
        for config in [&sync, &asynchronous] {
            let error = super::auth::build_auth_response(config, &challenge)
                .await
                .expect_err("reject before signing");
            assert_eq!(error.to_string(), "invalid vault authentication challenge");
        }
    }
    assert!(crate::ws::auth::validate_maker_auth_challenge(&canonical).is_err());
}

#[cfg(feature = "vault")]
#[tokio::test]
async fn canonical_vault_auth_signs_original_operator_challenge() {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let signer = Arc::new(BytesSigner::from_secret([5; 32]));
    let vault = crate::wire::encode_base58(&[8; 32]);
    let base = config("ws://localhost/vault".into());
    let config = ManagedWsConfig::new(
        "ws://localhost/vault",
        base.hello(),
        signer.clone(),
        crate::Address32::new(signer.pubkey_bytes()),
    )
    .with_vault_auth(
        crate::Address32::from_base58(&vault.clone()).expect("valid fixture"),
        crate::ProductSet::ALL,
    );
    let challenge = crate::ws::auth::test_vault_auth_challenge(&signer.pubkey_base58(), &vault);
    let response = super::auth::build_auth_response(&config, &challenge)
        .await
        .expect("canonical vault auth");
    assert_eq!(response.challenge, challenge);
    assert_eq!(
        response.pubkey,
        crate::Address32::new(signer.pubkey_bytes())
    );
    let signature = crate::wire::decode_base58_64(&response.signature).expect("signature");
    VerifyingKey::from_bytes(&signer.pubkey_bytes())
        .expect("key")
        .verify(challenge.as_bytes(), &Signature::from_bytes(&signature))
        .expect("original bytes");
}

use crate::ws::types::{
    ActiveRfqsData, AuthRequestData, AuthSuccessData, ClientMessage, HelloData,
    MakerPositionCapInfo, MakerQuoteScope, MmSummaryData, MyCapsData, MyQuotesMessage,
    ServerMessage, SubscribeAckData, SubscribeData, SubscriptionsMessage, UnsubscribeAckData,
    WelcomeData, WsChannel, WsErrorCode,
};

fn maker_auth() -> AuthOutcome {
    AuthOutcome { vault: None }
}

fn config(url: String) -> ManagedWsConfig {
    ManagedWsConfig::new(
        url,
        HelloData {
            protocol_version: crate::WS_PROTOCOL_VERSION.to_string(),
            on_disconnect: crate::ws::types::OnDisconnect::KeepQuotes,
            client_name: "managed-test".to_string(),
            client_version: "test".to_string(),
        },
        Arc::new(BytesSigner::from_secret([1u8; 32])),
        crate::Address32::new([135; 32]),
    )
    .with_initial_subscribe(SubscribeData {
        products: crate::ProductSet::ALL,
        request_id: uuid::Uuid::new_v4(),
        channels: Vec::new(),
        underlying_mints: vec![],
        quote_mints: vec![],
    })
}

#[tokio::test]
async fn credentials_expired_end_clears_resume_on_disconnect() {
    let authenticated_at = tokio::time::Instant::now();
    assert!(matches!(
        AuthenticatedSessionEnd::CredentialsExpired.finish(authenticated_at),
        SessionEnd::Disconnected {
            clear_resume: true,
            ..
        }
    ));
}

#[tokio::test]
async fn readiness_recovers_initial_target_before_authoritative_reads() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        let ClientMessage::GetSubscriptions(actual) = receive_client(&mut socket).await else {
            panic!("expected GetSubscriptions before recovery");
        };
        send_server(
            &mut socket,
            ServerMessage::Subscriptions(SubscriptionsMessage {
                request_id: actual.request_id,
                channels: Vec::new(),
                underlying_mints: vec![],
                quote_mints: vec![],
            }),
        )
        .await;
        let ClientMessage::Subscribe(subscribe) = receive_client(&mut socket).await else {
            panic!("expected desired Subscribe after GetSubscriptions");
        };
        assert_eq!(subscribe.channels, vec![WsChannel::Rfqs]);
        assert_eq!(subscribe.underlying_mints, Vec::new());
        assert_eq!(subscribe.quote_mints, Vec::new());
        send_server(
            &mut socket,
            ServerMessage::SubscribeAck(SubscribeAckData {
                request_id: subscribe.request_id,
                subscribed: subscribe.channels,
            }),
        )
        .await;
        let ClientMessage::GetActiveRfqs(active) = receive_client(&mut socket).await else {
            panic!("expected GetActiveRfqs after subscriptions recovery");
        };
        let ClientMessage::GetMyQuotes(my_quotes) = receive_client(&mut socket).await else {
            panic!("expected GetMyQuotes after subscriptions recovery");
        };
        send_server(
            &mut socket,
            ServerMessage::ActiveRfqs(ActiveRfqsData {
                request_id: active.request_id,
                rfqs: Vec::new(),
            }),
        )
        .await;
        send_server(
            &mut socket,
            ServerMessage::MyQuotes(MyQuotesMessage {
                request_id: my_quotes.request_id,
                quotes: Vec::new(),
                has_more: false,
            }),
        )
        .await;
    });

    let mut client = WsClient::connect(&format!("ws://{address}"))
        .await
        .expect("connect managed client");
    let config = config(format!("ws://{address}")).with_initial_subscribe(SubscribeData {
        products: crate::ProductSet::ALL,
        request_id: uuid::Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: vec![],
        quote_mints: vec![],
    });
    let (messages_tx, _messages_rx) = broadcast::channel(8);
    let mut sequence = 0;
    let mut inbound = InboundPublisher {
        tx: &messages_tx,
        connection_epoch: 1,
        sequence: &mut sequence,
    };

    let desired = DesiredSubscriptions::from_initial(config.initial_subscribe.as_ref());
    establish(&mut client, &config, &mut inbound, &desired, &maker_auth())
        .await
        .expect("ready quote session");
    server_task.await.expect("test server task");
}

#[tokio::test]
async fn readiness_rejects_a_partial_live_quote_snapshot() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        let ClientMessage::GetSubscriptions(actual) = receive_client(&mut socket).await else {
            panic!("expected GetSubscriptions first");
        };
        send_server(
            &mut socket,
            ServerMessage::Subscriptions(SubscriptionsMessage {
                request_id: actual.request_id,
                channels: vec![WsChannel::Trades],
                underlying_mints: vec![crate::Address32::new([160; 32])],
                quote_mints: vec![crate::Address32::new([160; 32])],
            }),
        )
        .await;
        let ClientMessage::Unsubscribe(unsubscribe) = receive_client(&mut socket).await else {
            panic!("expected removal of the stale channel");
        };
        assert_eq!(unsubscribe.channels, vec![WsChannel::Trades]);
        send_server(
            &mut socket,
            ServerMessage::UnsubscribeAck(UnsubscribeAckData {
                request_id: unsubscribe.request_id,
                unsubscribed: unsubscribe.channels,
            }),
        )
        .await;
        let ClientMessage::Subscribe(subscribe) = receive_client(&mut socket).await else {
            panic!("expected canonical desired Subscribe");
        };
        assert!(subscribe.channels.is_empty());
        assert_eq!(subscribe.underlying_mints, Vec::new());
        assert_eq!(subscribe.quote_mints, Vec::new());
        send_server(
            &mut socket,
            ServerMessage::SubscribeAck(SubscribeAckData {
                request_id: subscribe.request_id,
                subscribed: Vec::new(),
            }),
        )
        .await;
        let ClientMessage::GetActiveRfqs(active) = receive_client(&mut socket).await else {
            panic!("expected GetActiveRfqs first");
        };
        let ClientMessage::GetMyQuotes(my_quotes) = receive_client(&mut socket).await else {
            panic!("expected GetMyQuotes second");
        };
        assert_eq!(my_quotes.scope, MakerQuoteScope::Live {});
        send_server(
            &mut socket,
            ServerMessage::ActiveRfqs(ActiveRfqsData {
                request_id: active.request_id,
                rfqs: Vec::new(),
            }),
        )
        .await;
        send_server(
            &mut socket,
            ServerMessage::MyQuotes(MyQuotesMessage {
                request_id: my_quotes.request_id,
                quotes: Vec::new(),
                has_more: true,
            }),
        )
        .await;
        my_quotes.request_id
    });

    let mut client = WsClient::connect(&format!("ws://{address}"))
        .await
        .expect("connect managed client");
    let config = config(format!("ws://{address}"));
    let (messages_tx, _messages_rx) = broadcast::channel(8);
    let mut sequence = 0;
    let mut inbound = InboundPublisher {
        tx: &messages_tx,
        connection_epoch: 1,
        sequence: &mut sequence,
    };

    let desired = DesiredSubscriptions::from_initial(config.initial_subscribe.as_ref());
    let error = establish(&mut client, &config, &mut inbound, &desired, &maker_auth())
        .await
        .expect_err("a partial Live quote response fails the barrier");
    let my_quotes_id = server_task.await.expect("test server task");
    assert!(matches!(
        &error,
        SessionFailure::Retryable(detail)
            if detail.contains(&my_quotes_id.to_string()) && detail.contains("truncated")
    ));
}

#[tokio::test]
async fn data_readiness_reads_only_the_mm_summary() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        let ClientMessage::GetMmSummary(summary) = receive_client(&mut socket).await else {
            panic!("data readiness must not send GetSubscriptions or Subscribe");
        };
        send_server(
            &mut socket,
            ServerMessage::MmSummary(MmSummaryData {
                request_id: summary.request_id,
                maker_pda: crate::Address32::new([0; 32]),
                caps: MyCapsData {
                    request_id: summary.request_id,
                    positions: MakerPositionCapInfo {
                        current: 0,
                        limit: 0,
                    },
                    notional: Vec::new(),
                    balances: Vec::new(),
                    quote_liabilities: Vec::new(),
                },
                positions: Vec::new(),
                active_quotes: Vec::new(),
                markets: Vec::new(),
                tokens: Vec::new(),
                computed_at: std::time::SystemTime::UNIX_EPOCH,
                unrenderable_positions: Vec::new(),
                positions_has_more: false,
            }),
        )
        .await;
        socket
    });

    let mut client = WsClient::connect(&format!("ws://{address}"))
        .await
        .expect("connect managed client");
    let config = config(format!("ws://{address}")).with_endpoint(MakerWsEndpoint::Data);
    let (messages_tx, _messages_rx) = broadcast::channel(8);
    let mut sequence = 0;
    let mut inbound = InboundPublisher {
        tx: &messages_tx,
        connection_epoch: 1,
        sequence: &mut sequence,
    };
    let desired = DesiredSubscriptions::from_initial(config.initial_subscribe.as_ref());

    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        establish(&mut client, &config, &mut inbound, &desired, &maker_auth()),
    )
    .await
    .expect("one MmSummary completes data readiness")
    .expect("data readiness");
    let mut socket = server_task.await.expect("test server task");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), socket.next())
            .await
            .is_err(),
        "data readiness sent a read beyond GetMmSummary"
    );
}

async fn receive_client(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> ClientMessage {
    loop {
        let message = socket
            .next()
            .await
            .expect("client message")
            .expect("valid websocket message");
        let Message::Text(text) = message else {
            panic!("expected client text message");
        };
        let message: ClientMessage =
            serde_json::from_str(&text).expect("valid client protocol message");
        if matches!(message, ClientMessage::Ping) {
            send_server(
                socket,
                ServerMessage::Pong(crate::ws::types::PongData {
                    server_time_unix_ms: std::time::SystemTime::now(),
                }),
            )
            .await;
        } else {
            return message;
        }
    }
}

async fn send_server(
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

async fn send_welcome(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) {
    send_server(
        socket,
        ServerMessage::Welcome(WelcomeData {
            protocol_version: crate::WS_PROTOCOL_VERSION.to_string(),
            server_version: "test".to_string(),
            min_supported_version: crate::WS_PROTOCOL_VERSION.to_string(),

            server_time_unix_ms: std::time::UNIX_EPOCH,
            min_quote_validity_seconds: 100,
            settlement_buffer_seconds: 90,
        }),
    )
    .await;
}

#[tokio::test]
async fn a_panicked_session_task_publishes_session_panicked() {
    use crate::ws::managed::{ManagedWsError, ManagedWsState, spawn_managed_ws};

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        assert!(matches!(
            receive_client(&mut socket).await,
            ClientMessage::Hello(_)
        ));
        send_welcome(&mut socket).await;
        assert!(matches!(
            receive_client(&mut socket).await,
            ClientMessage::StartAuth(_)
        ));
        send_server(
            &mut socket,
            ServerMessage::AuthRequest(AuthRequestData {
                challenge: crate::ws::auth::TEST_MAKER_AUTH_CHALLENGE.to_string(),
            }),
        )
        .await;
        let _ = socket.next().await;
    });

    let config = ManagedWsConfig::new(
        format!("ws://{address}"),
        HelloData {
            protocol_version: crate::WS_PROTOCOL_VERSION.to_string(),
            on_disconnect: crate::ws::types::OnDisconnect::KeepQuotes,
            client_name: "managed-test".to_string(),
            client_version: "test".to_string(),
        },
        Arc::new(ExplodingSigner),
        crate::Address32::new([135; 32]),
    );
    let handle = spawn_managed_ws(config).expect("spawn managed session");
    let mut states = handle.subscribe_state();

    let reason = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = states.borrow_and_update().clone();
            if let ManagedWsState::Closed { reason } = state {
                return reason;
            }
            states.changed().await.expect("state channel");
        }
    })
    .await
    .expect("session must terminate after the panic");
    assert!(matches!(
        reason,
        ManagedWsTerminationReason::SessionPanicked
    ));

    let mut events = handle.subscribe_events();
    let mut disconnects = 0;
    loop {
        match events.recv().await.expect("event") {
            crate::ws::managed::ManagedWsEvent::Disconnected { close } => {
                assert!(close.is_none());
                disconnects += 1;
            }
            crate::ws::managed::ManagedWsEvent::Terminated(reason) => {
                assert_eq!(reason, ManagedWsTerminationReason::SessionPanicked);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(
        disconnects, 1,
        "a connected session that panics reports one disconnect"
    );

    assert!(matches!(
        handle.close().await,
        Err(ManagedWsError::TaskJoin(_))
    ));
    let _ = server_task.await;
}

async fn server_challenge_auth(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    session_id: &str,
) {
    assert!(matches!(
        receive_client(socket).await,
        ClientMessage::Hello(_)
    ));
    send_welcome(socket).await;
    assert!(matches!(
        receive_client(socket).await,
        ClientMessage::StartAuth(_)
    ));
    send_server(
        socket,
        ServerMessage::AuthRequest(AuthRequestData {
            challenge: crate::ws::auth::TEST_MAKER_AUTH_CHALLENGE.to_string(),
        }),
    )
    .await;
    assert!(matches!(
        receive_client(socket).await,
        ClientMessage::AuthChallenge(_)
    ));
    send_server(
        socket,
        ServerMessage::AuthSuccess(AuthSuccessData {
            session_id: session_id.to_string(),
            expires_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_900_000_000),
            maker_pda: None,
        }),
    )
    .await;
}

async fn complete_quote_readiness(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) {
    let ClientMessage::GetSubscriptions(actual) = receive_client(socket).await else {
        panic!("expected GetSubscriptions during quote readiness");
    };
    send_server(
        socket,
        ServerMessage::Subscriptions(SubscriptionsMessage {
            request_id: actual.request_id,
            channels: Vec::new(),
            underlying_mints: vec![],
            quote_mints: vec![],
        }),
    )
    .await;
    let ClientMessage::Subscribe(subscribe) = receive_client(socket).await else {
        panic!("expected Subscribe during quote readiness");
    };
    send_server(
        socket,
        ServerMessage::SubscribeAck(SubscribeAckData {
            request_id: subscribe.request_id,
            subscribed: subscribe.channels,
        }),
    )
    .await;
    let ClientMessage::GetActiveRfqs(active) = receive_client(socket).await else {
        panic!("expected GetActiveRfqs during quote readiness");
    };
    let ClientMessage::GetMyQuotes(my_quotes) = receive_client(socket).await else {
        panic!("expected GetMyQuotes during quote readiness");
    };
    send_server(
        socket,
        ServerMessage::ActiveRfqs(ActiveRfqsData {
            request_id: active.request_id,
            rfqs: Vec::new(),
        }),
    )
    .await;
    send_server(
        socket,
        ServerMessage::MyQuotes(MyQuotesMessage {
            request_id: my_quotes.request_id,
            quotes: Vec::new(),
            has_more: false,
        }),
    )
    .await;
}

#[tokio::test]
async fn pending_subscription_does_not_block_cancellation() {
    use crate::ws::managed::{ManagedWsError, SendAwaitError, spawn_managed_ws};
    use crate::ws::types::{AddMintsData, CancelAllQuotesAckMessage, CancelAllQuotesMessage};
    use std::time::Duration;

    async fn receive_command(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> ClientMessage {
        loop {
            match receive_client(socket).await {
                ClientMessage::Ping => {
                    send_server(
                        socket,
                        ServerMessage::Pong(crate::ws::types::PongData {
                            server_time_unix_ms: std::time::SystemTime::UNIX_EPOCH,
                        }),
                    )
                    .await;
                }
                command => return command,
            }
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server_challenge_auth(&mut socket, "session").await;
        complete_quote_readiness(&mut socket).await;
        let command = receive_command(&mut socket).await;
        assert!(
            matches!(command, ClientMessage::AddMints(_)),
            "expected AddMints, got {command:?}"
        );
        let command = receive_command(&mut socket).await;
        let ClientMessage::CancelAllQuotes(cancel) = command else {
            panic!("expected cancellation, got {command:?}");
        };
        send_server(
            &mut socket,
            ServerMessage::CancelAllQuotesAck(CancelAllQuotesAckMessage {
                request_id: cancel.request_id,
                cancelled_count: 0,
                cancelled_order_ids: Vec::new(),
            }),
        )
        .await;
        while socket.next().await.is_some() {}
    });
    let handle = spawn_managed_ws(config(format!("ws://{address}"))).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        handle.wait_until_ready().await.unwrap();
        let change = || ClientMessage::AddMints(AddMintsData {
            request_id: uuid::Uuid::new_v4(),
            underlying_mints: vec![crate::Address32::new([220; 32])],
            quote_mints: vec![],
        });
        handle.send(change()).await.unwrap();
        assert!(matches!(handle.send(change()).await, Err(ManagedWsError::SubscriptionPending)));
        assert!(matches!(handle.send_await(change(), Duration::from_secs(1)).await,
            Err(SendAwaitError::SubscriptionPending)));
        let request_id = uuid::Uuid::new_v4();
        let reply = handle.send_await(ClientMessage::CancelAllQuotes(CancelAllQuotesMessage {
            request_id,
            market: crate::ws::types::QueryFilter::All {},
        }), Duration::from_secs(1)).await.unwrap();
        assert!(matches!(&*reply, ServerMessage::CancelAllQuotesAck(ack) if ack.request_id == request_id));
    }).await.expect("subscription must not stall trading commands");
    handle.close().await.unwrap();
    server_task.await.unwrap();
}

#[tokio::test]
async fn a_dropped_session_reconnects_resumes_and_becomes_ready_again() {
    use crate::ws::managed::{ManagedWsState, spawn_managed_ws};

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept first connection");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        server_challenge_auth(&mut socket, "session-1").await;
        drop(socket);

        let (stream, _) = listener.accept().await.expect("accept reconnect");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        assert!(matches!(
            receive_client(&mut socket).await,
            ClientMessage::Hello(_)
        ));
        send_welcome(&mut socket).await;
        match receive_client(&mut socket).await {
            ClientMessage::ResumeAuth(resume) => assert_eq!(resume.session_id, "session-1"),
            other => panic!("expected ResumeAuth, got {other:?}"),
        }
        send_server(
            &mut socket,
            ServerMessage::AuthSuccess(AuthSuccessData {
                session_id: "session-2".to_string(),
                expires_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_900_000_000),
                maker_pda: None,
            }),
        )
        .await;
        complete_quote_readiness(&mut socket).await;
        while socket.next().await.is_some() {}
    });

    let handle = spawn_managed_ws(config(format!("ws://{address}"))).expect("spawn managed");
    let mut states = handle.subscribe_state();

    let epoch = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = states.borrow_and_update().clone();
            if let ManagedWsState::Ready { connection_epoch } = state
                && connection_epoch >= 2
            {
                return connection_epoch;
            }
            states.changed().await.expect("state channel");
        }
    })
    .await
    .expect("second Ready after reconnect");
    assert_eq!(epoch, 2);

    handle.close().await.expect("close managed session");
    server_task.await.expect("server assertions hold");
}

#[tokio::test]
async fn invalid_signature_terminates_only_after_three_consecutive_rejections() {
    use crate::ws::managed::{ManagedWsState, spawn_managed_ws};
    use crate::ws::types::AuthErrorData;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        for _ in 0..3 {
            let (stream, _) = listener.accept().await.expect("accept connection");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("websocket handshake");
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::Hello(_)
            ));
            send_welcome(&mut socket).await;
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::StartAuth(_)
            ));
            send_server(
                &mut socket,
                ServerMessage::AuthError(AuthErrorData {
                    reason: WsErrorCode::InvalidSignature,
                    message: "rejected".to_string(),
                }),
            )
            .await;
        }
    });

    let handle = spawn_managed_ws(config(format!("ws://{address}"))).expect("spawn managed");
    let mut states = handle.subscribe_state();

    let reason = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = states.borrow_and_update().clone();
            if let ManagedWsState::Closed { reason } = state {
                return reason;
            }
            states.changed().await.expect("state channel");
        }
    })
    .await
    .expect("terminal state after the third rejection");
    match reason {
        ManagedWsTerminationReason::AuthenticationRejected { reason, .. } => {
            assert_eq!(reason, "invalid_signature");
        }
        other => panic!("expected AuthenticationRejected, got {other:?}"),
    }
    server_task.await.expect("three connections were made");
}

#[tokio::test]
async fn reconnect_limit_during_strikes_reports_the_auth_rejection() {
    use crate::ws::managed::{ManagedWsState, spawn_managed_ws};
    use crate::ws::types::AuthErrorData;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.expect("accept connection");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("websocket handshake");
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::Hello(_)
            ));
            send_welcome(&mut socket).await;
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::StartAuth(_)
            ));
            send_server(
                &mut socket,
                ServerMessage::AuthError(AuthErrorData {
                    reason: WsErrorCode::InvalidSignature,
                    message: "rejected".to_string(),
                }),
            )
            .await;
        }
    });

    let mut config = config(format!("ws://{address}"));
    config.max_reconnect_attempts = 1;
    let handle = spawn_managed_ws(config).expect("spawn managed session");
    let mut states = handle.subscribe_state();

    let reason = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = states.borrow_and_update().clone();
            if let ManagedWsState::Closed { reason } = state {
                return reason;
            }
            states.changed().await.expect("state channel");
        }
    })
    .await
    .expect("terminal state at the reconnect limit");
    assert!(matches!(
        reason,
        ManagedWsTerminationReason::AuthenticationRejected { reason, .. }
            if reason == "invalid_signature"
    ));
    let _ = server_task.await;
}

#[tokio::test]
async fn invalid_mid_session_challenge_never_enters_writer_lane() {
    use crate::ws::error::WsClientError;
    use crate::ws::types::{AuthRequestData, ServerMessage};
    let base = config("ws://localhost/maker".into());
    let config = ManagedWsConfig::new(
        "ws://localhost/maker",
        base.hello(),
        Arc::new(ExplodingSigner),
        crate::Address32::new([135; 32]),
    );
    let (frames_tx, mut frames_rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = Box::pin(futures_util::sink::unfold(frames_tx, |tx, frame| {
        futures_util::future::ready(
            tx.send(frame)
                .map(|()| tx)
                .map_err(|_| WsClientError::ConnectionClosed),
        )
    }));
    let (events, _) = broadcast::channel(8);
    let (lanes, writer_task) =
        super::writer::spawn(sink, std::time::Duration::from_secs(1), 8, events.clone());
    let (_shutdown, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let (messages, _) = broadcast::channel(8);
    let mut sequence = 0;
    let mut inbound = InboundPublisher {
        tx: &messages,
        connection_epoch: 1,
        sequence: &mut sequence,
    };
    let mut tracker = super::AwaitTracker::new(8);
    let result = super::session_message::handle_ws_read(
        Some(Ok(ServerMessage::AuthRequest(AuthRequestData {
            challenge: "x".repeat(32),
        }))),
        &lanes,
        &config,
        &mut tracker,
        &mut inbound,
        &events,
        &mut shutdown_rx,
    )
    .await;
    assert!(matches!(
        result,
        Some((AuthenticatedSessionEnd::Disconnected, false))
    ));
    drop(lanes);
    writer_task.await.expect("writer drains and closes");
    assert!(
        frames_rx.recv().await.is_none(),
        "no auth response may be queued"
    );
}
