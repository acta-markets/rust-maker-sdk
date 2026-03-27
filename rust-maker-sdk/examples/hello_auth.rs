use acta_maker_sdk::ws::{client::WsClient, types::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;

    client
        .send_hello(HelloData {
            protocol_version: acta_maker_sdk::WS_PROTOCOL_VERSION.to_string(),
            features: vec!["quote_expired".to_string()],
            client_name: Some("maker-bot".to_string()),
            client_version: Some("0.1.0".to_string()),
        })
        .await?;

    while let Some(msg) = client.next().await {
        match msg? {
            ServerMessage::AuthRequest(data) => {
                // In real usage, sign `challenge` with your quote-signing keypair.
                let auth = AuthChallengeData {
                    challenge: data.challenge,
                    signature: "base58-signature".to_string(),
                    pubkey: "maker_signing_pubkey_base58".to_string(),
                };
                client.auth_challenge(auth).await?;
            }
            ServerMessage::AuthSuccess(data) => {
                println!(
                    "authenticated session: {} expires_at={:?}",
                    data.session_id, data.expires_at
                );
                break;
            }
            other => {
                println!("server: {other:?}");
            }
        }
    }

    Ok(())
}
