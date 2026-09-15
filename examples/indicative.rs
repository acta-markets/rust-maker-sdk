//! Respond to indicative price requests from the server.

mod common;

use acta_maker_sdk::ws::{client::WsClient, managed::normalize_maker_ws_url, types::*};
use acta_maker_sdk::{MarketId, Price};
use acta_maker_sdk::{SignerLike, WS_PROTOCOL_VERSION};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let common::Config { signer, owner, url } = common::load()?;
    let mut client = WsClient::connect(&normalize_maker_ws_url(&url)).await?;

    client
        .send_hello(HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            features: vec![],
            client_name: Some("maker-bot".to_string()),
            client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        })
        .await?;

    while let Some(msg) = client.next().await {
        match msg? {
            ServerMessage::Welcome(_) => client.start_auth(owner.clone()).await?,
            ServerMessage::AuthRequest(data) => {
                acta_maker_sdk::ws::auth::validate_maker_auth_challenge(&data.challenge)?;
                client
                    .auth_challenge(AuthChallengeData {
                        challenge: data.challenge.clone(),
                        signature: signer.sign_message_base58(data.challenge.as_bytes()),
                        pubkey: owner.clone(),
                    })
                    .await?;
            }
            ServerMessage::IndicativePricesRequest(req) => {
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
            ServerMessage::AuthError(error) => return Err(format!("{error:?}").into()),
            ServerMessage::VersionMismatch(error) => return Err(error.message.into()),
            ServerMessage::Error(error) => return Err(format!("{error:?}").into()),
            other => println!("server: {other:?}"),
        }
    }

    Ok(())
}
