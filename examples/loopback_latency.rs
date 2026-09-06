//! Loopback latency of the managed client: how long a command takes from
//! `try_send` to the peer's socket, and how long an inbound message takes from
//! the peer's socket to a subscriber. Run with `--release`.

use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{BytesSigner, WS_PROTOCOL_VERSION};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const OUTBOUND: usize = 20_000;
const INBOUND: usize = 5_000;
/// Inbound messages are paced so the number is the path of one message, not
/// the depth of a burst queue.
const INBOUND_PACING: Duration = Duration::from_millis(1);

type Socket = WebSocketStream<TcpStream>;

async fn server_send(socket: &mut Socket, message: &ServerMessage) {
    let text = serde_json::to_string(message).expect("serialize");
    socket
        .send(Message::Text(text.into()))
        .await
        .expect("peer send");
}

/// Next client frame; liveness pings are answered here so the session never
/// misses its pong deadline while a phase is measuring something else.
async fn client_frame(socket: &mut Socket) -> Option<ClientMessage> {
    loop {
        match socket.next().await? {
            Ok(Message::Text(text)) => match serde_json::from_str(&text).ok()? {
                ClientMessage::Ping => {
                    server_send(
                        socket,
                        &ServerMessage::Pong(PongData {
                            server_time_unix_ms: UNIX_EPOCH,
                        }),
                    )
                    .await;
                }
                message => return Some(message),
            },
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

async fn authenticate_peer(socket: &mut Socket) {
    assert!(matches!(
        client_frame(socket).await,
        Some(ClientMessage::Hello(_))
    ));
    server_send(
        socket,
        &ServerMessage::Welcome(WelcomeData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            server_version: "loopback".to_string(),
            min_supported_version: WS_PROTOCOL_VERSION.to_string(),
            enabled_features: vec![FEATURE_CANCEL_ON_DISCONNECT.to_string()],
            server_time_unix_ms: std::time::UNIX_EPOCH,
        }),
    )
    .await;
    assert!(matches!(
        client_frame(socket).await,
        Some(ClientMessage::StartAuth(_))
    ));
    server_send(
        socket,
        &ServerMessage::AuthRequest(AuthRequestData {
            challenge: "challenge".to_string(),
        }),
    )
    .await;
    assert!(matches!(
        client_frame(socket).await,
        Some(ClientMessage::AuthChallenge(_))
    ));
    server_send(
        socket,
        &ServerMessage::AuthSuccess(AuthSuccessData {
            session_id: "loopback".to_string(),
            expires_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_900_000_000),
            maker_pda: None,
        }),
    )
    .await;
    let mut reads = 0;
    while reads < 3 {
        let reply = match client_frame(socket).await.expect("recovery request") {
            ClientMessage::GetSubscriptions(m) => {
                ServerMessage::Subscriptions(SubscriptionsMessage {
                    request_id: m.request_id,
                    channels: vec![],
                    underlying_mints: None,
                    quote_mints: None,
                })
            }
            ClientMessage::Subscribe(m) => ServerMessage::SubscribeAck(SubscribeAckData {
                request_id: m.request_id,
                subscribed: m.channels,
            }),
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
        server_send(socket, &reply).await;
    }
}

fn percentiles(label: &str, samples: &mut [Duration]) {
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<28} n={:>6}  p50={:>7.1}µs  p90={:>7.1}µs  p99={:>7.1}µs  max={:>8.1}µs",
        samples.len(),
        at(0.50).as_secs_f64() * 1e6,
        at(0.90).as_secs_f64() * 1e6,
        at(0.99).as_secs_f64() * 1e6,
        samples[samples.len() - 1].as_secs_f64() * 1e6,
    );
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;

    let (outbound_started_tx, outbound_started_rx) = tokio::sync::oneshot::channel::<()>();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("handshake");
        authenticate_peer(&mut socket).await;
        eprintln!("peer: authenticated");

        // Phase 1: the client fires OUTBOUND data-lane queries; record each arrival.
        outbound_started_rx.await.expect("client started");
        let mut arrivals = Vec::with_capacity(OUTBOUND);
        while arrivals.len() < OUTBOUND {
            match client_frame(&mut socket).await {
                Some(ClientMessage::GetMarkets(_)) => arrivals.push(Instant::now()),
                Some(_) => {}
                None => panic!("client closed early"),
            }
        }

        eprintln!("peer: phase 1 done");
        // Phase 2: push INBOUND acknowledgements, recording the send instant.
        let mut sends = Vec::with_capacity(INBOUND);
        let mut next_send = tokio::time::Instant::now();
        for i in 0..INBOUND {
            let message = ServerMessage::QuoteAcknowledged(QuoteAcknowledgedMessage {
                rfq_id: Uuid::from_u128(i as u128),
                order_id: acta_maker_sdk::OrderId::new([0u8; 32]),
                replaced_order_id: None,
            });
            next_send += INBOUND_PACING;
            // Keep answering liveness pings while pacing.
            loop {
                tokio::select! {
                    () = tokio::time::sleep_until(next_send) => break,
                    frame = socket.next() => {
                        if let Some(Ok(Message::Text(text))) = frame
                            && matches!(serde_json::from_str(&text), Ok(ClientMessage::Ping))
                        {
                            server_send(
                                &mut socket,
                                &ServerMessage::Pong(PongData {
                                    server_time_unix_ms: UNIX_EPOCH,
                                }),
                            )
                            .await;
                        }
                    }
                }
            }
            sends.push(Instant::now());
            server_send(&mut socket, &message).await;
        }
        server_send(
            &mut socket,
            &ServerMessage::LogoutSuccess(LogoutSuccessData::default()),
        )
        .await;
        eprintln!("peer: phase 2 sent");
        while client_frame(&mut socket).await.is_some() {}
        eprintln!("peer: socket closed");
        (arrivals, sends)
    });

    let signer = Arc::new(BytesSigner::from_secret([1u8; 32]));
    let config = ManagedWsConfig::new(
        format!("ws://{address}"),
        HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            features: Vec::new(),
            client_name: Some("loopback-latency".to_string()),
            client_version: None,
        },
        signer,
    )
    .low_latency();
    let handle = spawn_managed_ws(config)?;
    let mut messages = handle.subscribe_messages();
    handle.wait_until_ready().await?;
    eprintln!("client: ready");

    // Phase 1: enqueue -> wire.
    // One frame in flight at a time: the number is the path of one frame from
    // `try_send` to the peer's socket, not the depth of a burst queue.
    let mut enqueued = Vec::with_capacity(OUTBOUND);
    outbound_started_tx.send(()).expect("peer waiting");
    for _ in 0..OUTBOUND {
        enqueued.push(Instant::now());
        handle
            .try_send(ClientMessage::GetMarkets(GetMarketsMessage {
                request_id: Uuid::nil(),
            }))?
            .wait()
            .await?;
    }
    eprintln!("client: written");

    // Phase 2: wire -> session parse -> subscriber.
    let mut parsed = Vec::with_capacity(INBOUND);
    let mut delivered = Vec::with_capacity(INBOUND);
    loop {
        let inbound = messages.recv().await?;
        match inbound.message() {
            ServerMessage::QuoteAcknowledged(_) => {
                parsed.push(inbound.received_at);
                delivered.push(Instant::now());
            }
            ServerMessage::LogoutSuccess(_) => break,
            _ => {}
        }
    }
    eprintln!("client: received");
    handle.close().await?;
    eprintln!("client: closed");
    let (arrivals, sends) = peer.await?;

    let mut enqueue_to_wire: Vec<Duration> = enqueued
        .iter()
        .zip(&arrivals)
        .map(|(sent, arrived)| arrived.saturating_duration_since(*sent))
        .collect();
    let mut wire_to_parsed: Vec<Duration> = sends
        .iter()
        .zip(&parsed)
        .map(|(sent, parsed)| parsed.saturating_duration_since(*sent))
        .collect();
    let mut parsed_to_subscriber: Vec<Duration> = parsed
        .iter()
        .zip(&delivered)
        .map(|(parsed, delivered)| delivered.saturating_duration_since(*parsed))
        .collect();

    println!(
        "loopback, low_latency profile, release build; one frame in flight, inbound paced at {:?}",
        INBOUND_PACING
    );
    percentiles("try_send -> peer socket", &mut enqueue_to_wire);
    percentiles("peer send -> session parsed", &mut wire_to_parsed);
    percentiles("session parsed -> subscriber", &mut parsed_to_subscriber);
    Ok(())
}
