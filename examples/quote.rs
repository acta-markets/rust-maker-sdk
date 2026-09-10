//! Subscribe to RFQs and submit signed quotes.

use acta_maker_sdk::ws::{client::WsClient, types::*};
use acta_maker_sdk::{
    AtomicNonceGenerator, BytesSigner, Nonce, Price, QuoteExpiry, RfqBinding, SignerLike,
    WS_PROTOCOL_VERSION,
};
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Example key. Never use it in production.
    let signer = BytesSigner::from_secret([1u8; 32]);

    let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;

    client
        .send_hello(HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            features: vec![],
            client_name: Some("maker-bot".to_string()),
            client_version: Some("0.1.0".to_string()),
        })
        .await?;

    while let Some(msg) = client.next().await {
        match msg? {
            ServerMessage::AuthRequest(data) => {
                client
                    .auth_challenge(AuthChallengeData {
                        challenge: data.challenge.clone(),
                        signature: signer.sign_message_base58(data.challenge.as_bytes()),
                        pubkey: signer.pubkey_base58(),
                    })
                    .await?;
            }
            ServerMessage::AuthSuccess(_) => {
                client
                    .subscribe(SubscribeData {
                        request_id: Uuid::new_v4(),
                        channels: vec![WsChannel::Rfqs],
                        underlying_mints: None,
                        quote_mints: None,
                    })
                    .await?;
            }
            ServerMessage::RfqBroadcast(rfq) => {
                // Quotes need 310 seconds of validity; 300 seconds are reserved for settlement.
                let valid_until = QuoteExpiry::after(std::time::Duration::from_secs(350))
                    .ok_or("system clock is before the Unix epoch")?;
                if valid_until.to_system_time() > rfq.market.expiry_ts {
                    continue;
                }

                let quote = RfqBinding::from_broadcast(&rfq)?
                    .quote()
                    .price(Price::new(1_000_000_000)) // your pricing logic here
                    .valid_until(valid_until)
                    .nonce(Nonce::new(NONCE_GEN.next_u64()?))
                    .sign(&signer)?;

                println!(
                    "submitting quote rfq={} strike={}",
                    quote.rfq_id, quote.strike
                );
                client.quote(quote).await?;
            }
            other => println!("server: {other:?}"),
        }
    }

    Ok(())
}
