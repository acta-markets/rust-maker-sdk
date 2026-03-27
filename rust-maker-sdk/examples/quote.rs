//! Subscribe to RFQs and send quotes with proper order_id computation and signing.

use acta_maker_sdk::ws::{client::WsClient, types::*};
use acta_maker_sdk::{
    AtomicNonceGenerator, BytesSigner, Nonce, OrderId, OrderPreimageArgs, Price, SignerLike,
    WS_PROTOCOL_VERSION, compute_order_id, decode_base58_32, encode_base58,
    sign_order_id_with_signer,
};
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load keypair — in production, read from file or env var.
    let keypair_bytes: [u8; 64] = [0u8; 64]; // placeholder
    let signer = BytesSigner::from_keypair(&keypair_bytes);

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
                // valid_until must be >= now + 310s.
                // Server applies 300s settlement buffer, so quote is tradeable
                // until valid_until - 300s.
                let valid_until =
                    std::time::SystemTime::now() + std::time::Duration::from_secs(350);
                let nonce = NONCE_GEN.next_u64();
                let price: u64 = 1_000_000_000; // your pricing logic here

                // Build order_id from canonical preimage and sign it.
                let args = OrderPreimageArgs {
                    chain_id: rfq.market.chain_id.value(),
                    program_id: decode_base58_32(&rfq.market.program_id)?,
                    is_taker_buy: true,
                    position_type: rfq.position_type as u8,
                    market: decode_base58_32(&rfq.market.market_pda)?,
                    strike: rfq.strike.value(),
                    quantity: rfq.quantity.value(),
                    gross_price: price,
                    valid_until: valid_until.duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                    maker: signer.pubkey_bytes(),
                    taker: decode_base58_32(&rfq.taker)?,
                    nonce,
                };

                let order_id = compute_order_id(&args);
                let signature = sign_order_id_with_signer(&order_id, &signer);

                client
                    .quote(QuoteMessage {
                        rfq_id: rfq.rfq_id,
                        strike: rfq.strike,
                        price: Price::new(price),
                        valid_until,
                        nonce: Nonce::new(nonce),
                        order_id: OrderId::new(order_id),
                        signature: encode_base58(&signature),
                    })
                    .await?;

                println!("quoted rfq={} strike={}", rfq.rfq_id, rfq.strike);
            }
            other => println!("server: {other:?}"),
        }
    }

    Ok(())
}
