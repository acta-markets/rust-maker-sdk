//! Respond to indicative price requests from the server.
//!
//! The server sends `IndicativePricesRequest` only to makers subscribed to the RFQ channel of
//! the requested side: `Rfqs` for taker sells, `TakerBuyRfqs` for taker buys. A request carries
//! no size: prices are per underlying unit, and the server treats them as stale after 60 s.

mod common;

use acta_maker_sdk::ws::{client::WsClient, managed::normalize_maker_ws_url, types::*};
use acta_maker_sdk::{MarketId, Price};
use acta_maker_sdk::{SignerLike, WS_PROTOCOL_VERSION};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let common::Config { signer, owner, url } = common::load()?;
    let owner = owner.ok_or("set ACTA_MAKER_OWNER to the registered maker owner")?;
    let mut client = WsClient::connect(&normalize_maker_ws_url(&url)).await?;

    let hello = HelloData {
        protocol_version: WS_PROTOCOL_VERSION.to_string(),
        on_disconnect: acta_maker_sdk::ws::types::OnDisconnect::KeepQuotes,
        client_name: "maker-bot".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    client.send_hello(hello.clone()).await?;

    while let Some(msg) = client.next().await {
        match msg? {
            ServerMessage::Welcome(_) => {
                client
                    .start_auth(acta_maker_sdk::Address32::from_base58(&owner.clone())?)
                    .await?;
            }
            ServerMessage::AuthRequest(data) => {
                acta_maker_sdk::ws::auth::validate_maker_auth_challenge(&data.challenge)?;
                client
                    .auth_challenge(AuthChallengeData {
                        challenge: data.challenge.clone(),
                        signature: signer.sign_message_base58(data.challenge.as_bytes()),
                        pubkey: acta_maker_sdk::Address32::from_base58(&owner.clone())?,
                    })
                    .await?;
            }
            ServerMessage::AuthSuccess(_) => {
                client
                    .subscribe(SubscribeData {
                        products: acta_maker_sdk::ProductSet::ALL,
                        request_id: uuid::Uuid::new_v4(),
                        channels: vec![WsChannel::Rfqs, WsChannel::TakerBuyRfqs],
                        underlying_mints: vec![],
                        quote_mints: vec![],
                    })
                    .await?;
            }
            ServerMessage::IndicativePricesRequest(req) => {
                let prices: Vec<IndicativeStrikePrice> = req
                    .strikes
                    .iter()
                    .map(|&strike| IndicativeStrikePrice {
                        strike,
                        price: Price::new(1_500_000_000),
                    })
                    .collect();

                client
                    .indicative_prices_response(IndicativePricesResponseMessage {
                        request_id: req.request_id,
                        market_pda: MarketId::new(req.market_descriptor.market_pda),
                        position_type: req.position_type,
                        side: req.side,
                        prices,
                    })
                    .await?;

                println!(
                    "responded to indicative request for {}",
                    req.market_descriptor.market_pda
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
