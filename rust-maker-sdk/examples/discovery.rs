use acta_maker_sdk::ws::{client::WsClient, types::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;

    client
        .send_hello(HelloData {
            protocol_version: acta_maker_sdk::WS_PROTOCOL_VERSION.to_string(),
            features: vec![],
            client_name: Some("maker-bot".to_string()),
            client_version: Some("0.1.0".to_string()),
        })
        .await?;

    while let Some(msg) = client.next().await {
        match msg? {
            ServerMessage::AuthRequest(data) => {
                let auth = AuthChallengeData {
                    challenge: data.challenge,
                    signature: "base58-signature".to_string(),
                    pubkey: "maker_signing_pubkey_base58".to_string(),
                };
                client.auth_challenge(auth).await?;
            }
            ServerMessage::AuthSuccess(_) => {
                client
                    .get_my_quotes(GetMyQuotesMessage {
                        request_id: uuid::Uuid::new_v4(),
                        active_only: true,
                        limit: None,
                    })
                    .await?;

                client
                    .get_maker_positions(GetMakerPositionsMessage {
                        request_id: uuid::Uuid::new_v4(),
                        market: None,
                        underlying_mint: None,
                        status: Some(vec!["open".to_string(), "funded".to_string()]),
                        min_expiry_ts: None,
                    })
                    .await?;

                client
                    .get_markets_for_maker(GetMarketsForMakerMessage {
                        request_id: uuid::Uuid::new_v4(),
                        underlying_mints: None,
                        quote_mints: None,
                        min_expiry_ts: None,
                        max_expiry_ts: None,
                        is_put: None,
                        include_stats: true,
                    })
                    .await?;

                client.get_maker_balances().await?;
            }
            other => println!("server: {:?}", other),
        }
    }

    Ok(())
}
