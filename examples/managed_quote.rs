//! Quote RFQs with `MakerQuoteClient`.

mod common;
#[path = "common/venue.rs"]
mod venue;

use acta_maker_sdk::ws::maker::MakerQuoteClient;
use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{
    AtomicNonceGenerator, Nonce, Price, QuoteBuildError, QuoteExpiry, RfqBinding,
    WS_PROTOCOL_VERSION,
};
use std::sync::Arc;
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let common::Config { signer, owner, url } = common::load()?;
    let owner = owner.ok_or("set ACTA_MAKER_OWNER to the registered maker owner")?;
    let maker_owner = acta_maker_sdk::decode_base58_32(&owner)?;
    let venue = venue::load(&url).await?;
    let lifetime = std::time::Duration::from_secs(u64::from(venue.min_quote_validity_seconds) + 60);
    let signer = Arc::new(signer);
    let signer_for_quotes = Arc::clone(&signer);

    let config = ManagedWsConfig::new(
        url,
        HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            features: vec!["quote_expired".to_string()],
            client_name: Some("maker-bot".to_string()),
            client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        },
        signer,
        owner,
    )
    .low_latency()
    .with_initial_subscribe(SubscribeData {
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs],
        underlying_mints: None,
        quote_mints: None,
    });

    let client = MakerQuoteClient::spawn(config)?;
    let mut rx = client.subscribe_messages();

    let mut events = client.subscribe_events();
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "session event logger lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            match event {
                ManagedWsEvent::Authenticated => tracing::info!("authenticated"),
                ManagedWsEvent::Ready => tracing::info!("ready"),
                ManagedWsEvent::Reconnecting { attempt, delay_ms } => {
                    tracing::warn!(attempt, delay_ms, "reconnecting");
                }
                ManagedWsEvent::Disconnected { close } => {
                    tracing::warn!(?close, "disconnected");
                }
                _ => {}
            }
        }
    });

    let epoch = client.wait_until_ready().await?;
    tracing::info!(epoch, "quote session ready");
    let mut recovery_epoch = epoch;
    let mut recovered = (false, false);

    loop {
        let msg = match rx.recv().await {
            Ok(msg) => msg,
            Err(ManagedReceiveError::Gap { skipped }) => {
                tracing::error!(
                    skipped,
                    "message stream lagged; refresh state before quoting"
                );
                return Err("managed message stream lagged".into());
            }
            Err(ManagedReceiveError::Closed) => break,
            Err(error) => return Err(format!("managed message stream failed: {error}").into()),
        };
        if msg.connection_epoch != recovery_epoch {
            recovery_epoch = msg.connection_epoch;
            recovered = (false, false);
        }
        match msg.message() {
            ServerMessage::ActiveRfqs(snapshot) => {
                tracing::info!(rfqs = snapshot.rfqs.len(), "RFQ recovery received");
                recovered.0 = true;
            }
            ServerMessage::MyQuotes(snapshot) => {
                tracing::info!(quotes = snapshot.quotes.len(), "quote recovery received");
                recovered.1 = true;
            }
            ServerMessage::RfqBroadcast(rfq) if recovered == (true, true) => {
                let valid_until =
                    QuoteExpiry::after(lifetime).ok_or("system clock is before the Unix epoch")?;
                let quote = match RfqBinding::from_broadcast(rfq)?
                    .quote()
                    .maker_owner(maker_owner)
                    .price(Price::new(1_000_000_000))
                    .valid_until(valid_until)
                    .venue(&venue)
                    .nonce(Nonce::new(NONCE_GEN.next_u64()?))
                    .sign(signer_for_quotes.as_ref())
                {
                    Ok(quote) => quote,
                    Err(QuoteBuildError::ValidUntilAfterMarketExpiry { .. }) => continue,
                    Err(error) => return Err(error.into()),
                };

                tracing::info!(rfq = %quote.rfq_id, strike = %quote.strike, "submitting quote");
                match client.quote_in_epoch(quote, recovery_epoch).await {
                    Ok(()) | Err(ManagedWsError::NotReady) => {}
                    Err(error) => return Err(error.into()),
                }
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
