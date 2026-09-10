use std::{future::Future, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

use super::auth::AuthenticationError;
use super::{
    AuthenticatedSessionEnd, InboundPublisher, SessionEnd, SessionFailure, auth::authenticate,
    ready::establish,
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
use crate::ws::types::{
    ActiveRfqsData, AuthErrorData, AuthRequestData, AuthSuccessData, ClientMessage,
    FEATURE_CANCEL_ON_DISCONNECT, HelloData, MakerPositionCapInfo, MakerQuoteScope, MmSummaryData,
    MyCapsData, MyQuotesMessage, RequestErrorEnvelope, ServerError, ServerMessage,
    SubscribeAckData, SubscribeData, SubscriptionsMessage, UnsubscribeAckData, WelcomeData,
    WsChannel,
};

fn config(url: String) -> ManagedWsConfig {
    ManagedWsConfig::new(
        url,
        HelloData {
            protocol_version: "1.0.0".to_string(),
            features: Vec::new(),
            client_name: Some("managed-test".to_string()),
            client_version: None,
        },
        Arc::new(BytesSigner::from_secret([1u8; 32])),
    )
    .with_auth_pubkey("maker")
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
                underlying_mints: None,
                quote_mints: None,
            }),
        )
        .await;
        let ClientMessage::Subscribe(subscribe) = receive_client(&mut socket).await else {
            panic!("expected desired Subscribe after GetSubscriptions");
        };
        assert_eq!(subscribe.channels, vec![WsChannel::Rfqs]);
        assert_eq!(subscribe.underlying_mints, Some(Vec::new()));
        assert_eq!(subscribe.quote_mints, Some(Vec::new()));
        send_server(
            &mut socket,
            ServerMessage::SubscribeAck(SubscribeAckData {
                request_id: subscribe.request_id,
                subscribed: subscribe.channels,
            }),
        )
        .await;
        let ClientMessage::GetMmSummary(summary) = receive_client(&mut socket).await else {
            panic!("expected GetMmSummary after subscriptions recovery");
        };
        let ClientMessage::GetActiveRfqs(active) = receive_client(&mut socket).await else {
            panic!("expected GetActiveRfqs after subscriptions recovery");
        };
        let ClientMessage::GetMyQuotes(my_quotes) = receive_client(&mut socket).await else {
            panic!("expected GetMyQuotes after subscriptions recovery");
        };
        send_server(
            &mut socket,
            ServerMessage::MmSummary(MmSummaryData {
                request_id: summary.request_id,
                maker_pda: String::new(),
                caps: MyCapsData {
                    request_id: uuid::Uuid::new_v4(),
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
                computed_at: std::time::SystemTime::UNIX_EPOCH,
                unrenderable_positions: Vec::new(),
                positions_has_more: false,
            }),
        )
        .await;
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
        request_id: uuid::Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: None,
        quote_mints: None,
    });
    let (messages_tx, _messages_rx) = broadcast::channel(8);
    let mut sequence = 0;
    let mut inbound = InboundPublisher {
        tx: &messages_tx,
        connection_epoch: 1,
        sequence: &mut sequence,
    };

    let desired = DesiredSubscriptions::from_initial(config.initial_subscribe.as_ref());
    establish(&mut client, &config, &mut inbound, &desired)
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
                underlying_mints: Some(vec!["stale".to_string()]),
                quote_mints: Some(vec!["stale".to_string()]),
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
        assert_eq!(subscribe.underlying_mints, Some(Vec::new()));
        assert_eq!(subscribe.quote_mints, Some(Vec::new()));
        send_server(
            &mut socket,
            ServerMessage::SubscribeAck(SubscribeAckData {
                request_id: subscribe.request_id,
                subscribed: Vec::new(),
            }),
        )
        .await;
        let ClientMessage::GetMmSummary(_) = receive_client(&mut socket).await else {
            panic!("expected required GetMmSummary");
        };
        let ClientMessage::GetActiveRfqs(active) = receive_client(&mut socket).await else {
            panic!("expected GetActiveRfqs second");
        };
        let ClientMessage::GetMyQuotes(my_quotes) = receive_client(&mut socket).await else {
            panic!("expected GetMyQuotes third");
        };
        assert_eq!(my_quotes.scope, MakerQuoteScope::Live);
        assert!(my_quotes.cursor.is_none());
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
    let error = establish(&mut client, &config, &mut inbound, &desired)
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
async fn data_readiness_skips_quote_subscription_recovery() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        let ClientMessage::GetMmSummary(_) = receive_client(&mut socket).await else {
            panic!("data readiness must not send GetSubscriptions or Subscribe");
        };
        let ClientMessage::GetActiveRfqs(_) = receive_client(&mut socket).await else {
            panic!("expected GetActiveRfqs");
        };
        let ClientMessage::GetMyQuotes(my_quotes) = receive_client(&mut socket).await else {
            panic!("expected GetMyQuotes");
        };
        send_server(
            &mut socket,
            ServerMessage::RequestError(RequestErrorEnvelope {
                request_id: my_quotes.request_id,
                error: ServerError::Generic {
                    code: "unavailable".to_string(),
                    message: "quotes projection is rebuilding".to_string(),
                },
            }),
        )
        .await;
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

    assert!(
        establish(&mut client, &config, &mut inbound, &desired)
            .await
            .is_err()
    );
    server_task.await.expect("test server task");
}

async fn receive_client(
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
            protocol_version: "1.0.0".to_string(),
            server_version: "test".to_string(),
            min_supported_version: "1.0.0".to_string(),
            enabled_features: vec![FEATURE_CANCEL_ON_DISCONNECT.to_string()],
            server_time_unix_ms: std::time::UNIX_EPOCH,
        }),
    )
    .await;
}

async fn send_welcome_without_cancel_on_disconnect(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) {
    send_server(
        socket,
        ServerMessage::Welcome(WelcomeData {
            protocol_version: "1.0.0".to_string(),
            server_version: "test".to_string(),
            min_supported_version: "1.0.0".to_string(),
            enabled_features: Vec::new(),
            server_time_unix_ms: std::time::UNIX_EPOCH,
        }),
    )
    .await;
}

async fn connect_and_authenticate<Server, ServerFuture>(
    server: Server,
    resume_session_id: &mut Option<String>,
) where
    Server: FnOnce(tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> ServerFuture
        + Send
        + 'static,
    ServerFuture: Future<Output = ()> + Send + 'static,
{
    try_connect_and_authenticate(|config| config, server, resume_session_id)
        .await
        .expect("authenticate managed client");
}

async fn try_connect_and_authenticate<Server, ServerFuture>(
    configure: impl FnOnce(ManagedWsConfig) -> ManagedWsConfig,
    server: Server,
    resume_session_id: &mut Option<String>,
) -> Result<(), AuthenticationError>
where
    Server: FnOnce(tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> ServerFuture
        + Send
        + 'static,
    ServerFuture: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept websocket");
        let socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");
        server(socket).await;
    });

    let mut client = WsClient::connect(&format!("ws://{address}"))
        .await
        .expect("connect managed client");
    let config = configure(config(format!("ws://{address}")));
    let (messages_tx, _messages_rx) = broadcast::channel(8);
    let mut sequence = 0;
    let mut inbound = InboundPublisher {
        tx: &messages_tx,
        connection_epoch: 1,
        sequence: &mut sequence,
    };

    let result = authenticate(&mut client, &config, &mut inbound, resume_session_id).await;
    server_task.await.expect("test server task");
    result
}

#[tokio::test]
async fn cancel_on_disconnect_is_requested_and_verified_in_welcome() {
    let mut resume_session_id = None;
    try_connect_and_authenticate(
        |config| config.with_cancel_on_disconnect(true),
        |mut socket| async move {
            match receive_client(&mut socket).await {
                ClientMessage::Hello(hello) => {
                    assert_eq!(
                        hello.features,
                        vec![FEATURE_CANCEL_ON_DISCONNECT.to_string()]
                    );
                }
                other => panic!("expected Hello, got {other:?}"),
            }
            send_server(
                &mut socket,
                ServerMessage::Welcome(WelcomeData {
                    protocol_version: "1.0.0".to_string(),
                    server_version: "test".to_string(),
                    min_supported_version: "1.0.0".to_string(),
                    enabled_features: vec![FEATURE_CANCEL_ON_DISCONNECT.to_string()],
                    server_time_unix_ms: std::time::UNIX_EPOCH,
                }),
            )
            .await;
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::StartAuth(_)
            ));
            send_server(
                &mut socket,
                ServerMessage::AuthRequest(AuthRequestData {
                    challenge: "challenge".to_string(),
                }),
            )
            .await;
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::AuthChallenge(_)
            ));
            send_server(
                &mut socket,
                ServerMessage::AuthSuccess(AuthSuccessData {
                    session_id: "cod".to_string(),
                    expires_at: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(1_900_000_000),
                    maker_pda: None,
                }),
            )
            .await;
        },
        &mut resume_session_id,
    )
    .await
    .expect("server enabled the feature");
    assert_eq!(resume_session_id.as_deref(), Some("cod"));
}

#[tokio::test]
async fn welcome_without_required_feature_terminates_before_auth() {
    let mut resume_session_id = None;
    let error = try_connect_and_authenticate(
        |config| config.with_cancel_on_disconnect(true),
        |mut socket| async move {
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::Hello(_)
            ));
            send_welcome_without_cancel_on_disconnect(&mut socket).await;
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    receive_client(&mut socket)
                )
                .await
                .is_err(),
                "no StartAuth after a Welcome that lacks the feature"
            );
        },
        &mut resume_session_id,
    )
    .await
    .expect_err("missing feature is terminal");
    assert!(matches!(
        error.termination_reason(),
        Some(ManagedWsTerminationReason::FeatureUnsupported { feature })
            if feature == FEATURE_CANCEL_ON_DISCONNECT
    ));
}

#[tokio::test]
async fn managed_auth_resumes_then_falls_back_only_when_expired() {
    let mut resume_session_id = None;
    connect_and_authenticate(
        |mut socket| async move {
            match receive_client(&mut socket).await {
                ClientMessage::Hello(hello) => {
                    assert_eq!(hello.protocol_version, "1.0.0");
                    assert_eq!(hello.client_name.as_deref(), Some("managed-test"));
                    assert_eq!(hello.client_version, None);
                    assert_eq!(
                        hello.features,
                        vec![FEATURE_CANCEL_ON_DISCONNECT.to_string()]
                    );
                }
                other => panic!("expected Hello, got {other:?}"),
            }
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(25),
                    receive_client(&mut socket)
                )
                .await
                .is_err(),
                "authentication must wait for Welcome"
            );
            send_welcome(&mut socket).await;
            match receive_client(&mut socket).await {
                ClientMessage::StartAuth(start) => assert_eq!(start.pubkey, "maker"),
                other => panic!("expected StartAuth, got {other:?}"),
            }
            send_server(
                &mut socket,
                ServerMessage::AuthRequest(AuthRequestData {
                    challenge: "challenge".to_string(),
                }),
            )
            .await;
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::AuthChallenge(_)
            ));
            send_server(
                &mut socket,
                ServerMessage::AuthSuccess(AuthSuccessData {
                    session_id: "resume-me".to_string(),
                    expires_at: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(1_900_000_000),
                    maker_pda: None,
                }),
            )
            .await;
        },
        &mut resume_session_id,
    )
    .await;
    assert_eq!(resume_session_id.as_deref(), Some("resume-me"));

    connect_and_authenticate(
        |mut socket| async move {
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::Hello(_)
            ));
            send_welcome(&mut socket).await;
            match receive_client(&mut socket).await {
                ClientMessage::ResumeAuth(data) => {
                    assert_eq!(data.session_id, "resume-me");
                }
                other => panic!("expected resume auth, got {other:?}"),
            }
            send_server(
                &mut socket,
                ServerMessage::AuthSuccess(AuthSuccessData {
                    session_id: "resumed".to_string(),
                    expires_at: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(1_900_000_000),
                    maker_pda: None,
                }),
            )
            .await;
        },
        &mut resume_session_id,
    )
    .await;
    assert_eq!(resume_session_id.as_deref(), Some("resumed"));

    connect_and_authenticate(
        |mut socket| async move {
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::Hello(_)
            ));
            send_welcome(&mut socket).await;
            send_server(
                &mut socket,
                ServerMessage::AuthRequest(AuthRequestData {
                    challenge: "fresh-challenge".to_string(),
                }),
            )
            .await;
            assert!(matches!(
                receive_client(&mut socket).await,
                ClientMessage::ResumeAuth(_)
            ));
            send_server(
                &mut socket,
                ServerMessage::AuthError(AuthErrorData {
                    reason: "session_expired".to_string(),
                    message: None,
                }),
            )
            .await;
            match receive_client(&mut socket).await {
                ClientMessage::AuthChallenge(data) => {
                    assert_eq!(data.challenge, "fresh-challenge");
                }
                other => panic!("expected auth challenge response, got {other:?}"),
            }
            send_server(
                &mut socket,
                ServerMessage::AuthSuccess(AuthSuccessData {
                    session_id: "fresh".to_string(),
                    expires_at: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(1_900_000_000),
                    maker_pda: None,
                }),
            )
            .await;
        },
        &mut resume_session_id,
    )
    .await;
    assert_eq!(resume_session_id.as_deref(), Some("fresh"));
}

#[tokio::test]
async fn a_panicked_session_task_publishes_session_panicked() {
    use crate::ws::managed::{
        ManagedWsError, ManagedWsState, ManagedWsTerminationReason, spawn_managed_ws,
    };

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
                challenge: "challenge".to_string(),
            }),
        )
        .await;
        let _ = socket.next().await;
    });

    let config = ManagedWsConfig::new(
        format!("ws://{address}"),
        HelloData {
            protocol_version: "1.0.0".to_string(),
            features: Vec::new(),
            client_name: Some("managed-test".to_string()),
            client_version: None,
        },
        Arc::new(ExplodingSigner),
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
            challenge: "challenge".to_string(),
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
            underlying_mints: None,
            quote_mints: None,
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
    let ClientMessage::GetMmSummary(summary) = receive_client(socket).await else {
        panic!("expected GetMmSummary during quote readiness");
    };
    let ClientMessage::GetActiveRfqs(active) = receive_client(socket).await else {
        panic!("expected GetActiveRfqs during quote readiness");
    };
    let ClientMessage::GetMyQuotes(my_quotes) = receive_client(socket).await else {
        panic!("expected GetMyQuotes during quote readiness");
    };
    send_server(
        socket,
        ServerMessage::MmSummary(MmSummaryData {
            request_id: summary.request_id,
            maker_pda: String::new(),
            caps: MyCapsData {
                request_id: uuid::Uuid::new_v4(),
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
            computed_at: std::time::SystemTime::UNIX_EPOCH,
            unrenderable_positions: Vec::new(),
            positions_has_more: false,
        }),
    )
    .await;
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
        // Leave AddMints unacknowledged. CancelAll must still reach the socket.
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
            underlying_mints: Some(vec!["mint".to_owned()]),
            quote_mints: None,
        });
        handle.send(change()).await.unwrap();
        assert!(matches!(handle.send(change()).await, Err(ManagedWsError::SubscriptionPending)));
        assert!(matches!(handle.send_await(change(), Duration::from_secs(1)).await,
            Err(SendAwaitError::SubscriptionPending)));
        let request_id = uuid::Uuid::new_v4();
        let reply = handle.send_await(ClientMessage::CancelAllQuotes(CancelAllQuotesMessage {
            request_id,
            market: None,
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
    use crate::ws::managed::{ManagedWsState, ManagedWsTerminationReason, spawn_managed_ws};
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
                    reason: "invalid_signature".to_string(),
                    message: None,
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
    // All three accepts happened; a single-strike terminal would leave the
    // server task waiting and this join hanging.
    server_task.await.expect("three connections were made");
}

#[tokio::test]
async fn reconnect_limit_during_strikes_reports_the_auth_rejection() {
    use crate::ws::managed::{ManagedWsState, ManagedWsTerminationReason, spawn_managed_ws};
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
                    reason: "invalid_signature".to_string(),
                    message: None,
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
