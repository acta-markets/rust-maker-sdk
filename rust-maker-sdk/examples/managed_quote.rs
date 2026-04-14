//! Full maker bot using ManagedWs (recommended for production).
//!
//! Connects, subscribes, quotes on every RFQ, handles lifecycle events.
//! ManagedWs handles reconnection and re-authentication automatically.

use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{
    AtomicNonceGenerator, BytesSigner, Nonce, OrderId, OrderPreimageArgs, Price, SignerLike,
    WS_PROTOCOL_VERSION, compute_order_id, decode_base58_32, encode_base58,
    sign_order_id_with_signer,
};
use std::sync::Arc;
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Load keypair — in production, read from file or env var.
    let keypair_bytes: [u8; 64] = [0u8; 64]; // placeholder
    let signer = BytesSigner::from_keypair(&keypair_bytes);
    let signer_for_auth = signer.clone();
    let signer_for_quotes = signer.clone();

    // Configure managed connection.
    let mut config = ManagedWsConfig::new(
        "wss://devnet-api.acta.markets/maker",
        HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            features: vec!["quote_expired".to_string()],
            client_name: Some("maker-bot".to_string()),
            client_version: Some("0.1.0".to_string()),
        },
        signer.pubkey_base58(),
        Arc::new(move |challenge: &str| {
            Ok(signer_for_auth.sign_message_base58(challenge.as_bytes()))
        }),
    );

    // Subscribe to RFQs on every connect/reconnect.
    config.initial_subscribe = Some(SubscribeData {
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: None,
        quote_mints: None,
    });

    let handle = spawn_managed_ws(config);

    // Monitor connection events in a separate task.
    let mut events = handle.subscribe_events();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                ManagedWsEvent::Authenticated => tracing::info!("authenticated"),
                ManagedWsEvent::Reconnecting { attempt, delay_ms } => {
                    tracing::warn!(attempt, delay_ms, "reconnecting");
                }
                ManagedWsEvent::Disconnected => tracing::warn!("disconnected"),
                _ => {}
            }
        }
    });

    // Main message loop.
    let mut rx = handle.subscribe_messages();
    while let Ok(msg) = rx.recv().await {
        match msg.as_ref() {
            ServerMessage::RfqBroadcast(rfq) => {
                let valid_until =
                    std::time::SystemTime::now() + std::time::Duration::from_secs(350);
                let nonce = NONCE_GEN.next_u64();
                let price: u64 = 1_000_000_000; // your pricing logic

                let args = OrderPreimageArgs {
                    chain_id: rfq.market.chain_id.value(),
                    program_id: decode_base58_32(&rfq.market.program_id).unwrap(),
                    is_taker_buy: false,
                    position_type: rfq.position_type as u8,
                    market: decode_base58_32(&rfq.market.market_pda).unwrap(),
                    strike: rfq.strike.value(),
                    quantity: rfq.quantity.value(),
                    gross_price: price,
                    valid_until: valid_until
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    maker: signer_for_quotes.pubkey_bytes(),
                    taker: decode_base58_32(&rfq.taker).unwrap(),
                    nonce,
                };

                let order_id = compute_order_id(&args);
                let signature = sign_order_id_with_signer(&order_id, &signer_for_quotes);

                handle
                    .send(ClientMessage::Quote(QuoteMessage {
                        rfq_id: rfq.rfq_id,
                        strike: rfq.strike,
                        price: Price::new(price),
                        valid_until,
                        nonce: Nonce::new(nonce),
                        order_id: OrderId::new(order_id),
                        signature: encode_base58(&signature),
                    }))
                    .await?;

                tracing::info!(rfq = %rfq.rfq_id, strike = %rfq.strike, "quoted");
            }
            ServerMessage::QuoteAcknowledged(ack) => {
                tracing::info!(rfq = %ack.rfq_id, order = ?ack.order_id, "ack");
            }
            ServerMessage::QuoteBestStatus(status) => {
                tracing::info!(rfq = %status.rfq_id, best = status.is_best, "best status");
            }
            ServerMessage::QuoteOutbid(outbid) => {
                tracing::info!(
                    rfq = %outbid.rfq_id,
                    ours = outbid.your_price.value(),
                    best = ?outbid.current_best_price.map(|p| p.value()),
                    "outbid"
                );
                // Consider sending ReplaceQuote with a better price here.
            }
            ServerMessage::QuoteFilled(fill) => {
                tracing::info!(
                    rfq = %fill.rfq_id,
                    position = %fill.position_pda,
                    tx = %fill.tx_signature,
                    "filled"
                );
            }
            ServerMessage::RfqClosed(closed) => {
                tracing::info!(rfq = %closed.rfq_id, reason = ?closed.reason, "rfq closed");
            }
            ServerMessage::QuoteRejected(rejected) => {
                tracing::warn!(rfq = %rejected.rfq_id, reason = ?rejected.reason, "rejected");
            }
            ServerMessage::Error(err) => {
                tracing::error!(?err, "session error");
            }
            ServerMessage::RequestError(envelope) => {
                tracing::error!(
                    request_id = %envelope.request_id,
                    error = ?envelope.error,
                    "request error"
                );
            }
            _ => {}
        }
    }

    Ok(())
}
