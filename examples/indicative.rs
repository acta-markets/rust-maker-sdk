//! Respond to indicative price requests from the server.

use acta_maker_sdk::ws::{client::WsClient, types::*};
use acta_maker_sdk::{BytesSigner, SignerLike, WS_PROTOCOL_VERSION};
use acta_maker_sdk::{MarketId, Price};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
            ServerMessage::IndicativePricesRequest(req) => {
                // Return a price for each requested strike.
                let prices: Vec<IndicativeStrikePrice> = req
                    .strikes
                    .iter()
                    .map(|&strike| IndicativeStrikePrice {
                        strike,
                        price: Price::new(1_500_000_000), // your pricing logic
                    })
                    .collect();

                client
                    .indicative_prices_response(IndicativePricesResponseMessage {
                        request_id: req.request_id,
                        market: MarketId::new(req.market.market_pda.clone()),
                        position_type: req.position_type,
                        prices,
                    })
                    .await?;

                println!(
                    "responded to indicative request for {}",
                    req.market.market_pda
                );
            }
            other => println!("server: {other:?}"),
        }
    }

    Ok(())
}
