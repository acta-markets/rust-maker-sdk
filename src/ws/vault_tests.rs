use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use super::tx::ensure_deadline_ahead_at;
use super::{VaultOperatorClient, VaultOperatorConfig, VaultRequestError, VaultTxError};
use crate::orders::{BytesSigner, SignerLike};
use crate::types::ids::OrderId;
use crate::types::{
    VaultDelegatePermissions, VaultOperatorKind, VaultSwapDirection, VaultSwapMode,
};
use crate::wire::{decode_base58_32, decode_base58_64, encode_base58};
use crate::ws::managed::{ManagedWsEvent, ManagedWsState, SendAwaitError};
use crate::ws::types::{
    ActiveRfqsData, AuthRequestData, ClientMessage, HelloData, MakerPositionCapInfo, MmSummaryData,
    MyActiveRfqsData, MyCapsData, MyQuotesMessage, PongData, PositionsData, ServerMessage,
    SwapQuoteData, SwapQuoteResultData, VaultAuthSuccessData, VaultProjectionAdvancedData,
    WelcomeData,
};

type Socket = WebSocketStream<TcpStream>;

const OPERATOR_SECRET: [u8; 32] = [2u8; 32];
const VAULT_BYTES: [u8; 32] = [9u8; 32];

fn operator() -> Arc<BytesSigner> {
    Arc::new(BytesSigner::from_secret(OPERATOR_SECRET))
}

fn vault_pda() -> String {
    encode_base58(&VAULT_BYTES)
}

fn config(url: String) -> VaultOperatorConfig {
    VaultOperatorConfig::new(
        url,
        HelloData {
            protocol_version: "1.0.0".to_string(),
            features: Vec::new(),
            client_name: Some("vault-test".to_string()),
            client_version: None,
        },
        vault_pda(),
        operator(),
    )
    .with_request_timeout(Duration::from_millis(200))
    .with_managed(|mut managed| {
        managed.reconnect_delay = Duration::from_millis(10);
        managed.max_reconnect_delay = Duration::from_millis(20);
        managed.reconnect_jitter_ratio = 0.0;
        managed
    })
}

async fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test websocket");
    let address = listener.local_addr().expect("test websocket address");
    (listener, format!("ws://{address}"))
}

async fn accept(listener: &TcpListener) -> Socket {
    let (stream, _) = listener.accept().await.expect("accept websocket");
    tokio_tungstenite::accept_async(stream)
        .await
        .expect("websocket handshake")
}

/// Next protocol message; keepalive pings are answered here so tests only
/// see traffic they caused.
async fn receive_client(socket: &mut Socket) -> ClientMessage {
    loop {
        let message = socket
            .next()
            .await
            .expect("client message")
            .expect("valid websocket message");
        let Message::Text(text) = message else {
            panic!("expected client text message, got {message:?}");
        };
        let parsed: ClientMessage =
            serde_json::from_str(&text).expect("valid client protocol message");
        if matches!(parsed, ClientMessage::Ping) {
            send_server(
                socket,
                ServerMessage::Pong(PongData {
                    server_time_unix_ms: std::time::UNIX_EPOCH,
                }),
            )
            .await;
            continue;
        }
        return parsed;
    }
}

async fn send_server(socket: &mut Socket, message: ServerMessage) {
    socket
        .send(Message::Text(
            serde_json::to_string(&message)
                .expect("serialize server message")
                .into(),
        ))
        .await
        .expect("send server message");
}

/// Hello -> Welcome -> StartVaultAuth -> AuthRequest -> AuthChallenge -> VaultAuthSuccess.
async fn vault_handshake(socket: &mut Socket, permissions: VaultDelegatePermissions) {
    vault_handshake_response(
        socket,
        ServerMessage::VaultAuthSuccess(VaultAuthSuccessData {
            vault_pda: vault_pda(),
            operator: operator().pubkey_base58(),
            operator_kind: VaultOperatorKind::Delegate,
            permissions,
        }),
    )
    .await;
}

async fn vault_handshake_response(socket: &mut Socket, response: ServerMessage) {
    let ClientMessage::Hello(hello) = receive_client(socket).await else {
        panic!("expected Hello");
    };
    assert!(
        !hello
            .features
            .iter()
            .any(|feature| feature == crate::ws::types::FEATURE_CANCEL_ON_DISCONNECT)
    );
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
    let start = match receive_client(socket).await {
        ClientMessage::StartVaultAuth(start) => start,
        other => panic!("expected StartVaultAuth, got {other:?}"),
    };
    let operator_pubkey = operator().pubkey_base58();
    assert_eq!(start.operator, operator_pubkey);
    assert_eq!(start.vault_pda, vault_pda());

    let challenge = crate::ws::auth::test_vault_auth_challenge(&operator_pubkey, &vault_pda());
    send_server(
        socket,
        ServerMessage::AuthRequest(AuthRequestData {
            challenge: challenge.clone(),
        }),
    )
    .await;
    let answer = match receive_client(socket).await {
        ClientMessage::AuthChallenge(answer) => answer,
        other => panic!("expected AuthChallenge, got {other:?}"),
    };
    assert_eq!(answer.challenge, challenge);
    assert_eq!(answer.pubkey, operator_pubkey);
    let key = VerifyingKey::from_bytes(&decode_base58_32(&answer.pubkey).expect("pubkey"))
        .expect("verifying key");
    let signature = Signature::from_bytes(&decode_base58_64(&answer.signature).expect("sig"));
    key.verify(challenge.as_bytes(), &signature)
        .expect("challenge signed by the operator key");

    send_server(socket, response).await;
}

/// Answer reconcile reads in the order they arrive, asserting the expected set.
async fn answer_reconcile(socket: &mut Socket, expected: &[&str]) {
    for expected_type in expected {
        let read = receive_client(socket).await;
        let actual: &'static str = (&read).into();
        assert_eq!(actual, *expected_type, "unexpected reconcile read {read:?}");
        let reply = match read {
            ClientMessage::GetMyActiveRfqs(m) => ServerMessage::MyActiveRfqs(MyActiveRfqsData {
                request_id: m.request_id,
                rfqs: Vec::new(),
            }),
            ClientMessage::GetPositions(m) => ServerMessage::Positions(PositionsData {
                request_id: m.request_id,
                positions: Vec::new(),
            }),
            ClientMessage::GetMmSummary(m) => ServerMessage::MmSummary(MmSummaryData {
                request_id: m.request_id,
                maker_pda: vault_pda(),
                caps: MyCapsData {
                    request_id: m.request_id,
                    positions: MakerPositionCapInfo {
                        current: 0,
                        limit: 10,
                    },
                    notional: Vec::new(),
                    balances: Vec::new(),
                },
                positions: Vec::new(),
                active_quotes: Vec::new(),
                markets: Vec::new(),
                tokens: Vec::new(),
                computed_at: std::time::UNIX_EPOCH,
                unrenderable_positions: Vec::new(),
                positions_has_more: false,
            }),
            ClientMessage::GetActiveRfqs(m) => ServerMessage::ActiveRfqs(ActiveRfqsData {
                request_id: m.request_id,
                rfqs: Vec::new(),
            }),
            ClientMessage::GetMyQuotes(m) => {
                assert!(m.scope == crate::ws::types::MakerQuoteScope::Live && m.cursor.is_none());
                ServerMessage::MyQuotes(MyQuotesMessage {
                    request_id: m.request_id,
                    quotes: Vec::new(),
                    has_more: false,
                })
            }
            other => panic!("unexpected reconcile read {other:?}"),
        };
        send_server(socket, reply).await;
    }
}

const TAKER_READS: &[&str] = &["get_my_active_rfqs", "get_positions"];
const ALL_READS: &[&str] = &[
    "get_my_active_rfqs",
    "get_positions",
    "get_mm_summary",
    "get_active_rfqs",
    "get_my_quotes",
];

async fn drain_until_closed(mut socket: Socket) {
    while let Some(Ok(frame)) = socket.next().await {
        if matches!(frame, Message::Close(_)) {
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vault_auth_handshake_reaches_ready_with_taker_reconcile() {
    let (listener, url) = bind().await;
    let (allow_reads, reads_allowed) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_TAKER).await;
        reads_allowed.await.unwrap();
        answer_reconcile(&mut socket, TAKER_READS).await;
        drain_until_closed(socket).await;
    });

    let client = VaultOperatorClient::spawn(config(url)).expect("spawn vault client");
    let mut events = client.subscribe_events();
    let mut grants = client.subscribe_vault_auth();
    tokio::time::timeout(Duration::from_secs(2), grants.wait_for(Option::is_some))
        .await
        .unwrap()
        .unwrap();
    assert!(client.auth().is_some());
    assert!(!client.can_maker() && !client.can_taker());
    assert!(matches!(
        client.swap_quote(swap_request()).await,
        Err(VaultRequestError::Session(
            crate::ws::managed::ManagedWsError::NotReady
        ))
    ));
    allow_reads.send(()).unwrap();
    client
        .wait_until_ready()
        .await
        .expect("vault session ready");

    let auth = client.auth().expect("vault auth recorded");
    assert_eq!(auth.permissions, VaultDelegatePermissions::OPEN_TAKER);
    assert_eq!(auth.operator, operator().pubkey_base58());
    assert!(client.can_taker());
    assert!(!client.can_maker());

    let mut saw = Vec::new();
    while let Ok(event) = events.try_recv() {
        saw.push(std::mem::discriminant(&event));
        if matches!(event, ManagedWsEvent::Ready) {
            break;
        }
    }
    let expected = [
        ManagedWsEvent::Connected,
        ManagedWsEvent::Authenticated,
        ManagedWsEvent::Reconciled,
        ManagedWsEvent::Ready,
    ]
    .map(|event| std::mem::discriminant(&event));
    assert_eq!(saw, expected);

    client.close().await.expect("close");
    assert!(client.auth().is_none());
    assert!(grants.borrow().is_none());
    assert!(!client.can_maker() && !client.can_taker());
    peer.await.expect("peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconcile_adds_maker_reads_when_delegate_can_sign_quotes() {
    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        drain_until_closed(socket).await;
    });

    let client = VaultOperatorClient::spawn(config(url)).expect("spawn vault client");
    client
        .wait_until_ready()
        .await
        .expect("vault session ready");
    assert!(client.can_maker() && client.can_taker());

    client.close().await.expect("close");
    peer.await.expect("peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn governance_without_quote_permission_recovers_only_taker_state() {
    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake_response(
            &mut socket,
            ServerMessage::VaultAuthSuccess(VaultAuthSuccessData {
                vault_pda: vault_pda(),
                operator: operator().pubkey_base58(),
                operator_kind: VaultOperatorKind::Governance,
                permissions: VaultDelegatePermissions::from_bits(61),
            }),
        )
        .await;
        answer_reconcile(&mut socket, TAKER_READS).await;
        drain_until_closed(socket).await;
    });

    let client = VaultOperatorClient::spawn(config(url)).expect("spawn vault client");
    client
        .wait_until_ready()
        .await
        .expect("governance recovery");
    assert!(!client.can_maker());
    assert!(client.can_taker());
    client.close().await.expect("close");
    peer.await.expect("peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn governance_with_quote_permission_recovers_maker_state_and_can_quote() {
    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake_response(
            &mut socket,
            ServerMessage::VaultAuthSuccess(VaultAuthSuccessData {
                vault_pda: vault_pda(),
                operator: operator().pubkey_base58(),
                operator_kind: VaultOperatorKind::Governance,
                permissions: VaultDelegatePermissions::ALL,
            }),
        )
        .await;
        answer_reconcile(&mut socket, ALL_READS).await;
        drain_until_closed(socket).await;
    });
    let client = VaultOperatorClient::spawn(config(url)).expect("spawn vault client");
    client
        .wait_until_ready()
        .await
        .expect("governance maker recovery");
    assert!(client.can_maker() && client.can_taker());
    client.close().await.expect("close");
    peer.await.expect("peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_reauthenticates_with_start_vault_auth() {
    let (listener, url) = bind().await;
    let (first_ready_observed, allow_disconnect) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_TAKER).await;
        answer_reconcile(&mut socket, TAKER_READS).await;
        allow_disconnect.await.expect("first ready state observed");
        drop(socket);

        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_TAKER).await;
        answer_reconcile(&mut socket, TAKER_READS).await;
        drain_until_closed(socket).await;
    });

    let client = VaultOperatorClient::spawn(config(url)).expect("spawn vault client");
    let first = client
        .wait_until_ready()
        .await
        .expect("first session ready");
    assert_eq!(first, 1);

    let mut state = client.raw().subscribe_state();
    first_ready_observed
        .send(())
        .expect("peer awaits disconnect");
    tokio::time::timeout(Duration::from_secs(5), async {
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
    .expect("second session ready");

    client.close().await.expect("close");
    peer.await.expect("peer");
}

fn swap_request() -> SwapQuoteData {
    SwapQuoteData {
        request_id: Uuid::new_v4(),
        vault_pda: vault_pda(),
        mode: VaultSwapMode::Rebalance,
        direction: VaultSwapDirection::MainToSecond,
        amount: "1000".to_string(),
        slippage_bps: 50,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn swap_quote_is_correlated_and_times_out_without_a_reply() {
    let (listener, url) = bind().await;
    let (unanswered_tx, unanswered_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;

        let ClientMessage::SwapQuote(_) = receive_client(&mut socket).await else {
            panic!("expected SwapQuote");
        };
        unanswered_rx.await.expect("client timed out");

        let ClientMessage::SwapQuote(request) = receive_client(&mut socket).await else {
            panic!("expected SwapQuote");
        };
        send_server(
            &mut socket,
            ServerMessage::SwapQuoteResult(SwapQuoteResultData {
                request_id: request.request_id,
                execution_id: OrderId::new([4u8; 32]),
                mode: request.mode,
                tx_base64: String::new(),
                last_valid_block_height: 1,
                in_amount: request.amount.clone(),
                quoted_out_amount: "990".to_string(),
                minimum_out_amount: "985".to_string(),
                valid_until: "0".to_string(),
                main_price_update: "main-update".into(),
                second_price_update: "second-update".into(),
            }),
        )
        .await;
        drain_until_closed(socket).await;
    });

    let client = VaultOperatorClient::spawn(config(url)).expect("spawn vault client");
    client
        .wait_until_ready()
        .await
        .expect("vault session ready");

    let error = client
        .swap_quote(swap_request())
        .await
        .expect_err("no reply must time out");
    assert!(matches!(
        error,
        VaultRequestError::Transport(SendAwaitError::Timeout)
    ));
    unanswered_tx.send(()).expect("peer waits for the timeout");

    let request = swap_request();
    let result = client
        .swap_quote(request.clone())
        .await
        .expect("correlated swap quote");
    assert_eq!(result.request_id, request.request_id);
    assert_eq!(result.execution_id, OrderId::new([4u8; 32]));

    client.close().await.expect("close");
    peer.await.expect("peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paused_swap_requests_receive_their_own_rejections_out_of_order() {
    use crate::ws::types::{RequestErrorEnvelope, ServerError, WsErrorCode};

    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let mut ids = Vec::new();
        for _ in 0..2 {
            let ClientMessage::SwapQuote(request) = receive_client(&mut socket).await else {
                panic!("expected SwapQuote");
            };
            ids.push(request.request_id);
        }
        for request_id in ids.into_iter().rev() {
            send_server(
                &mut socket,
                ServerMessage::RequestError(RequestErrorEnvelope {
                    request_id,
                    error: ServerError::Generic {
                        code: WsErrorCode::TradingPaused,
                        message: "Trading is paused".into(),
                    },
                    delivery: None,
                }),
            )
            .await;
        }
        drain_until_closed(socket).await;
    });
    let client = VaultOperatorClient::spawn(config(url)).unwrap();
    client.wait_until_ready().await.unwrap();
    let first = swap_request();
    let mut second = swap_request();
    second.request_id = Uuid::new_v4();
    let expected = [first.request_id, second.request_id];
    let (first_result, second_result) =
        tokio::join!(client.swap_quote(first), client.swap_quote(second));
    for (result, expected_id) in [first_result, second_result].into_iter().zip(expected) {
        let Err(VaultRequestError::Rejected {
            request_id,
            response,
        }) = result
        else {
            panic!("pause must reject the request, not time out");
        };
        assert_eq!(request_id, expected_id);
        assert!(
            matches!(response.as_ref(), ServerMessage::RequestError(error)
            if matches!(&error.error, ServerError::Generic { code: WsErrorCode::TradingPaused, .. }))
        );
    }
    client.close().await.unwrap();
    peer.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trading_commands_resolve_on_their_receipt_or_rejection() {
    use crate::types::ids::{MarketId, PositionType, Quantity, Strike, TimeoutSeconds};
    use crate::ws::types::{
        AcceptQuoteMessage, CommandAcceptedData, Delivery, RequestErrorEnvelope, RfqRequestMessage,
        ServerError, SubmitSignedSponsoredTxData, SwapExecutionState, SwapStatusMessage,
    };

    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let ClientMessage::RfqRequest(rfq) = receive_client(&mut socket).await else {
            panic!("expected RfqRequest");
        };
        let created = serde_json::json!({
            "type": "RfqCreated",
            "data": {
                "rfq_id": Uuid::new_v4(),
                "rfq_version": 1,
                "request_id": rfq.request_id,
                "expires_at": 1,
                "created_at": 1,
            },
        });
        socket
            .send(Message::Text(created.to_string().into()))
            .await
            .expect("send RfqCreated");
        send_server(
            &mut socket,
            ServerMessage::CommandAccepted(CommandAcceptedData {
                request_id: rfq.request_id,
            }),
        )
        .await;
        let ClientMessage::AcceptQuote(accept) = receive_client(&mut socket).await else {
            panic!("expected AcceptQuote");
        };
        send_server(
            &mut socket,
            ServerMessage::RequestError(RequestErrorEnvelope {
                request_id: accept.request_id,
                error: ServerError::QuoteNotFound,
                delivery: None,
            }),
        )
        .await;
        for delivery in [Delivery::NotForwarded, Delivery::Unknown] {
            let ClientMessage::SubmitSignedSponsoredTx(submit) = receive_client(&mut socket).await
            else {
                panic!("expected SubmitSignedSponsoredTx");
            };
            send_server(
                &mut socket,
                ServerMessage::RequestError(RequestErrorEnvelope {
                    request_id: submit.request_id,
                    error: ServerError::InternalError,
                    delivery: Some(delivery),
                }),
            )
            .await;
        }
        let ClientMessage::SubmitSignedSponsoredTx(submit) = receive_client(&mut socket).await
        else {
            panic!("expected SubmitSignedSponsoredTx");
        };
        send_server(
            &mut socket,
            ServerMessage::CommandAccepted(CommandAcceptedData {
                request_id: submit.request_id,
            }),
        )
        .await;
        let ClientMessage::GetSwapStatus(status) = receive_client(&mut socket).await else {
            panic!("expected GetSwapStatus");
        };
        send_server(
            &mut socket,
            ServerMessage::SwapStatus(SwapStatusMessage {
                request_id: status.request_id,
                execution_id: status.execution_id,
                state: SwapExecutionState::Pending,
            }),
        )
        .await;
        drain_until_closed(socket).await;
    });

    let client = VaultOperatorClient::spawn(config(url)).expect("spawn");
    client.wait_until_ready().await.expect("ready");
    client
        .rfq_request(RfqRequestMessage {
            request_id: Uuid::new_v4(),
            market: MarketId::new("market"),
            position_type: PositionType::CoveredCall,
            strike: Strike::new(1),
            quantity: Quantity::new(1),
            timeout_seconds: TimeoutSeconds::new(30),
        })
        .await
        .expect("RfqRequest accepted");
    let rejected = client
        .accept_quote(AcceptQuoteMessage {
            request_id: Uuid::new_v4(),
            rfq_id: Uuid::new_v4(),
            maker: "maker".into(),
            order_id: OrderId::new([5; 32]),
        })
        .await
        .expect_err("AcceptQuote rejected");
    assert!(matches!(
        rejected.server_error(),
        Some(ServerError::QuoteNotFound)
    ));
    assert_eq!(rejected.delivery(), None);
    let submit = || SubmitSignedSponsoredTxData {
        request_id: Uuid::new_v4(),
        order_id: OrderId::new([6; 32]),
        tx_base64: "tx".into(),
    };
    for delivery in [Delivery::NotForwarded, Delivery::Unknown] {
        let rejected = client
            .submit_signed_sponsored_tx(submit())
            .await
            .expect_err("submit rejected");
        assert_eq!(rejected.delivery(), Some(delivery));
    }
    client
        .submit_signed_sponsored_tx(submit())
        .await
        .expect("submit accepted");
    let status = client
        .swap_status(OrderId::new([7; 32]))
        .await
        .expect("swap status");
    assert_eq!(status.execution_id, OrderId::new([7; 32]));
    assert_eq!(status.state, SwapExecutionState::Pending);
    client.close().await.expect("close");
    peer.await.expect("peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vault_projection_advanced_reaches_the_typed_subscriber() {
    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_TAKER).await;
        answer_reconcile(&mut socket, TAKER_READS).await;
        send_server(
            &mut socket,
            ServerMessage::VaultProjectionAdvanced(VaultProjectionAdvancedData {
                vault_pda: vault_pda(),
                version: 7,
                slot: 100,
                transaction_index: 1,
                signature: "sig".to_string(),
                event_index: 0,
            }),
        )
        .await;
        drain_until_closed(socket).await;
    });

    let client = VaultOperatorClient::spawn(config(url)).expect("spawn vault client");
    let mut projections = client.subscribe_projections();
    client
        .wait_until_ready()
        .await
        .expect("vault session ready");

    let step = tokio::time::timeout(Duration::from_secs(5), projections.recv())
        .await
        .expect("projection within deadline")
        .expect("projection delivered");
    assert_eq!(step.version(), 7);
    assert_eq!(step.connection_epoch, 1);
    assert!(step.follows(6));
    assert!(!step.follows(5));

    client.close().await.expect("close");
    peer.await.expect("peer");
}

fn assert_signed_by(signed_base64: &str, slot: usize, signer: &BytesSigner) {
    let bytes = STANDARD.decode(signed_base64).expect("base64");
    let count = usize::from(bytes[0]);
    let message = &bytes[1 + 64 * count..];
    let signature = Signature::from_bytes(
        bytes[1 + 64 * slot..1 + 64 * (slot + 1)]
            .try_into()
            .expect("64-byte signature"),
    );
    VerifyingKey::from_bytes(&signer.pubkey_bytes())
        .expect("verifying key")
        .verify(message, &signature)
        .expect("signature over the message bytes");
}

#[test]
fn deadlines_are_checked_before_signing() {
    assert_eq!(
        ensure_deadline_ahead_at("submit_before", "100", 99).expect("ahead"),
        100
    );
    assert!(matches!(
        ensure_deadline_ahead_at("submit_before", "100", 100),
        Err(VaultTxError::Expired {
            field: "submit_before",
            deadline: 100,
            now: 100
        })
    ));
    assert!(matches!(
        ensure_deadline_ahead_at("valid_until", "soon", 1),
        Err(VaultTxError::InvalidDeadline {
            field: "valid_until",
            ..
        })
    ));
}

#[tokio::test]
async fn raw_vault_actions_require_ready() {
    let (listener, url) = bind().await;
    let client = VaultOperatorClient::spawn(config(url)).expect("spawn");
    for message in [
        ClientMessage::SwapQuote(swap_request()),
        ClientMessage::CancelRfq(crate::ws::types::CancelRfqData {
            request_id: Uuid::new_v4(),
            rfq_id: Uuid::new_v4(),
        }),
    ] {
        assert!(matches!(
            client.raw().send(message).await,
            Err(crate::ws::managed::ManagedWsError::NotReady)
        ));
    }
    client.close().await.expect("close");
    drop(listener);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vault_unsubscribe_survives_fresh_authentication() {
    use crate::ws::types::{UnsubscribeAckData, UnsubscribeData, WsChannel};
    let (listener, url) = bind().await;
    let (acked, may_disconnect) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_TAKER).await;
        answer_reconcile(&mut socket, TAKER_READS).await;
        let ClientMessage::Unsubscribe(request) = receive_client(&mut socket).await else {
            panic!("expected user unsubscribe");
        };
        assert_eq!(request.channels, vec![WsChannel::Vaults]);
        send_server(
            &mut socket,
            ServerMessage::UnsubscribeAck(UnsubscribeAckData {
                request_id: request.request_id,
                unsubscribed: request.channels,
            }),
        )
        .await;
        may_disconnect.await.unwrap();
        drop(socket);
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_TAKER).await;
        let ClientMessage::Unsubscribe(request) = receive_client(&mut socket).await else {
            panic!("fresh auth must restore the user's removed feed before Ready");
        };
        assert_eq!(request.channels, vec![WsChannel::Vaults]);
        send_server(
            &mut socket,
            ServerMessage::UnsubscribeAck(UnsubscribeAckData {
                request_id: request.request_id,
                unsubscribed: request.channels,
            }),
        )
        .await;
        answer_reconcile(&mut socket, TAKER_READS).await;
        drain_until_closed(socket).await;
    });
    let client = VaultOperatorClient::spawn(config(url)).unwrap();
    client.wait_until_ready().await.unwrap();
    let mut state = client.raw().subscribe_state();
    client
        .raw()
        .send_await(
            ClientMessage::Unsubscribe(UnsubscribeData {
                request_id: Uuid::new_v4(),
                channels: vec![WsChannel::Vaults],
            }),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    acked.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                *state.borrow(),
                ManagedWsState::Ready {
                    connection_epoch: 2
                }
            ) {
                break;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    client.close().await.unwrap();
    peer.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_clears_old_grant_then_uses_reduced_permissions() {
    for invalidate in [false, true] {
        let (listener, url) = bind().await;
        let (disconnect, disconnect_allowed) = oneshot::channel();
        let (reauth, reauth_allowed) = oneshot::channel();
        let peer = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
            answer_reconcile(&mut socket, ALL_READS).await;
            disconnect_allowed.await.unwrap();
            let _old_socket = if invalidate {
                send_server(
                    &mut socket,
                    ServerMessage::Error(crate::ws::types::ServerError::Generic {
                        code: crate::ws::types::WsErrorCode::VaultSessionInvalidated,
                        message: "vault operator grant changed".into(),
                    }),
                )
                .await;
                Some(socket)
            } else {
                drop(socket);
                None
            };
            let mut socket = accept(&listener).await;
            reauth_allowed.await.unwrap();
            vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_TAKER).await;
            answer_reconcile(&mut socket, TAKER_READS).await;
            drain_until_closed(socket).await;
        });
        let client = VaultOperatorClient::spawn(config(url)).unwrap();
        client.wait_until_ready().await.unwrap();
        assert!(client.can_maker() && client.can_taker());
        let mut grants = client.subscribe_vault_auth();
        let mut state = client.raw().subscribe_state();
        disconnect.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), grants.wait_for(Option::is_none))
            .await
            .unwrap()
            .unwrap();
        assert!(client.auth().is_none());
        assert!(!client.can_maker() && !client.can_taker());
        reauth.send(()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            state.wait_for(|state| {
                matches!(
                    state,
                    ManagedWsState::Ready {
                        connection_epoch: 2
                    }
                )
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            client.auth().unwrap().permissions,
            VaultDelegatePermissions::OPEN_TAKER
        );
        assert!(!client.can_maker() && client.can_taker());
        client.close().await.unwrap();
        peer.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_reauthentication_does_not_retain_the_old_grant() {
    let (listener, url) = bind().await;
    let (disconnect, disconnect_allowed) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        disconnect_allowed.await.unwrap();
        drop(socket);
        let mut socket = accept(&listener).await;
        vault_handshake_response(
            &mut socket,
            ServerMessage::AuthError(crate::ws::types::AuthErrorData {
                reason: crate::ws::types::WsErrorCode::VaultOperatorUnauthorized,
                message: None,
            }),
        )
        .await;
        drain_until_closed(socket).await;
    });
    let client = VaultOperatorClient::spawn(config(url)).unwrap();
    client.wait_until_ready().await.unwrap();
    let mut state = client.raw().subscribe_state();
    disconnect.send(()).unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        state.wait_for(|state| matches!(state, ManagedWsState::Closed { .. })),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(client.state(), ManagedWsState::Closed {
        reason: crate::ws::managed::ManagedWsTerminationReason::AuthenticationRejected { ref reason, .. }
    } if reason == "vault_operator_unauthorized"));
    assert!(client.auth().is_none());
    assert!(client.permissions().is_none());
    assert!(!client.can_maker() && !client.can_taker());
    client.close().await.unwrap();
    peer.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_initial_reads_clear_the_authenticated_grant() {
    let (listener, url) = bind().await;
    let (fail_reads, failure_allowed) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        failure_allowed.await.unwrap();
        let ClientMessage::GetMyActiveRfqs(request) = receive_client(&mut socket).await else {
            panic!("expected initial read");
        };
        send_server(
            &mut socket,
            ServerMessage::RequestError(crate::ws::types::RequestErrorEnvelope {
                request_id: request.request_id,
                error: crate::ws::types::ServerError::Generic {
                    code: crate::ws::types::WsErrorCode::Unknown("unavailable".into()),
                    message: "projection unavailable".into(),
                },
                delivery: None,
            }),
        )
        .await;
        drain_until_closed(socket).await;
    });
    let client = VaultOperatorClient::spawn(config(url)).unwrap();
    let mut grants = client.subscribe_vault_auth();
    tokio::time::timeout(Duration::from_secs(2), grants.wait_for(Option::is_some))
        .await
        .unwrap()
        .unwrap();
    assert!(!client.can_maker() && !client.can_taker());
    fail_reads.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), grants.wait_for(Option::is_none))
        .await
        .unwrap()
        .unwrap();
    assert!(client.auth().is_none());
    assert!(!client.can_maker() && !client.can_taker());
    client.close().await.unwrap();
    peer.await.unwrap();
}

#[tokio::test]
async fn checked_signing_verifies_served_intent_before_signing() {
    use std::str::FromStr;

    use solana_sdk::message::AddressLookupTableAccount;
    use solana_sdk::pubkey::Pubkey;

    use crate::chain::vault::{
        ComputeBudgetBounds, PairPriceFeeds, VaultIntentError, VaultManager, VaultManagerArgs,
        VaultOpenIntent, VaultSwapIntent, VaultTradeTerms,
    };

    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/vault_served_tx.json")).unwrap();
    let key = |value: &serde_json::Value| Pubkey::from_str(value.as_str().unwrap()).unwrap();
    let manager = |case: &serde_json::Value, field: &str| {
        let vault = &case[field];
        VaultManager::new(VaultManagerArgs {
            program_id: key(&case["program_id"]),
            vault_id: hex::decode(vault["vault_id"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
            actor: key(&vault["actor"]),
            delegate_authority: key(&vault["delegate_authority"]),
            main_mint: key(&vault["main_mint"]),
            second_mint: key(&vault["second_mint"]),
            share_mint: key(&vault["share_mint"]),
            main_token_program: key(&vault["main_token_program"]),
            second_token_program: key(&vault["second_token_program"]),
        })
        .unwrap()
    };
    let table = |case: &serde_json::Value| {
        let table = &case["served"]["lookup_table"];
        vec![AddressLookupTableAccount {
            key: key(&table["key"]),
            addresses: table["addresses"]
                .as_array()
                .unwrap()
                .iter()
                .map(key)
                .collect(),
        }]
    };
    let bounds = ComputeBudgetBounds {
        max_unit_limit: 400_000,
        max_unit_price_micro_lamports: 1_000,
    };
    let spawn = |url: String, secret: u8| {
        let mut config = config(url);
        config.operator = Arc::new(BytesSigner::from_secret([secret; 32]));
        VaultOperatorClient::spawn(config).expect("spawn vault client")
    };

    let case = &cases[0];
    let (_listener, url) = bind().await;
    let client = spawn(url, 10);
    let writer = manager(case, "writer");
    let p = &case["position"];
    let intent = VaultOpenIntent::Sale {
        writer: &writer,
        maker_owner: key(&case["maker_owner"]),
        maker_quote_signer: key(&p["quote_signer"]),
        terms: VaultTradeTerms {
            underlying_mint: key(&p["underlying_mint"]),
            quote_mint: key(&p["quote_mint"]),
            expiry_ts: p["expiry_ts"].as_u64().unwrap(),
            is_put: p["is_put"].as_bool().unwrap(),
            strike: p["strike"].as_u64().unwrap(),
            quantity: p["quantity"].as_u64().unwrap(),
            price: p["price"].as_u64().unwrap(),
            valid_until: p["valid_until"].as_u64().unwrap(),
            nonce: p["nonce"].as_u64().unwrap(),
        },
    };
    let mut to_sign = crate::ws::types::SponsoredTxToSignData {
        order_id: OrderId::from_hex(p["order_id"].as_str().unwrap()).unwrap(),
        tx_base64: case["served"]["tx_base64"].as_str().unwrap().to_owned(),
        last_valid_block_height: 1,
        signature_deadline: std::time::SystemTime::now() + Duration::from_secs(60),
    };
    let signed = client
        .verify_and_sign_sponsored_tx(&to_sign, &intent, &table(case), bounds)
        .unwrap();
    assert_signed_by(&signed.tx_base64, 0, &BytesSigner::from_secret([10; 32]));
    assert!(matches!(
        client.verify_and_sign_sponsored_tx(&to_sign, &intent, &[], bounds),
        Err(VaultTxError::Intent(VaultIntentError::MissingLookupTable(
            _
        )))
    ));
    let expired = crate::ws::types::SponsoredTxToSignData {
        signature_deadline: std::time::UNIX_EPOCH,
        ..to_sign.clone()
    };
    assert!(matches!(
        client.verify_and_sign_sponsored_tx(&expired, &intent, &table(case), bounds),
        Err(VaultTxError::Expired { .. })
    ));
    to_sign.order_id = OrderId::new([1; 32]);
    assert!(matches!(
        client.verify_and_sign_sponsored_tx(&to_sign, &intent, &table(case), bounds),
        Err(VaultTxError::Intent(VaultIntentError::OrderId))
    ));
    client.close().await.expect("close");

    let case = &cases[14];
    let (_listener, url) = bind().await;
    let client = spawn(url, 60);
    let vault = manager(case, "swapper");
    let swap = &case["swap"];
    let request = SwapQuoteData {
        request_id: Uuid::from_str(swap["request_id"].as_str().unwrap()).unwrap(),
        vault_pda: vault.addresses.vault.to_string(),
        mode: VaultSwapMode::Rebalance,
        direction: VaultSwapDirection::MainToSecond,
        amount: "1000000".into(),
        slippage_bps: 50,
    };
    let quote = SwapQuoteResultData {
        request_id: request.request_id,
        execution_id: OrderId::new([4; 32]),
        mode: VaultSwapMode::Rebalance,
        tx_base64: case["served"]["tx_base64"].as_str().unwrap().to_owned(),
        last_valid_block_height: 1,
        in_amount: "1000000".into(),
        quoted_out_amount: "2000".into(),
        minimum_out_amount: "1990".into(),
        valid_until: "1000100".into(),
        main_price_update: swap["main_price_update"].as_str().unwrap().to_owned(),
        second_price_update: swap["second_price_update"].as_str().unwrap().to_owned(),
    };
    let feed = |name: &str| {
        let mut id = [0; 32];
        hex::decode_to_slice(swap[name].as_str().unwrap(), &mut id).unwrap();
        id
    };
    let intent = |quote| VaultSwapIntent {
        vault: &vault,
        request: &request,
        quote,
        price_feeds: PairPriceFeeds {
            main: feed("main_feed"),
            second: feed("second_feed"),
            shards: &[0, 1],
        },
        router: crate::chain::vault::JUPITER_V6_PROGRAM_ID,
        min_output_floor: 1990,
    };
    assert!(matches!(
        client.verify_and_sign_swap(&intent(&quote), &table(case), bounds),
        Err(VaultTxError::Expired { .. })
    ));
    let mut loose = quote.clone();
    loose.minimum_out_amount = "1".into();
    assert!(matches!(
        client.verify_and_sign_swap(&intent(&loose), &table(case), bounds),
        Err(VaultTxError::Intent(VaultIntentError::Terms(_)))
    ));
    client.close().await.expect("close");
}

#[cfg(feature = "chain-rpc")]
#[path = "vault_sale_tests.rs"]
mod sale;
