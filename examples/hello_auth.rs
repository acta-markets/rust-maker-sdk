mod common;

use acta_maker_sdk::SignerLike;
use acta_maker_sdk::ws::{client::WsClient, managed::normalize_maker_ws_url, types::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let common::Config { signer, owner, url } = common::load()?;
    let mut client = WsClient::connect(&normalize_maker_ws_url(&url)).await?;

    client
        .send_hello(HelloData {
            protocol_version: acta_maker_sdk::WS_PROTOCOL_VERSION.to_string(),
            features: vec!["quote_expired".to_string()],
            client_name: Some("maker-bot".to_string()),
            client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        })
        .await?;

    while let Some(msg) = client.next().await {
        match msg? {
            ServerMessage::Welcome(_) => client.start_auth(owner.clone()).await?,
            ServerMessage::AuthRequest(data) => {
                acta_maker_sdk::ws::auth::validate_maker_auth_challenge(&data.challenge)?;
                let auth = AuthChallengeData {
                    signature: signer.sign_message_base58(data.challenge.as_bytes()),
                    challenge: data.challenge,
                    pubkey: owner.clone(),
                };
                client.auth_challenge(auth).await?;
            }
            ServerMessage::AuthSuccess(data) => {
                println!(
                    "authenticated maker: {owner} expires_at={:?}",
                    data.expires_at
                );
                client.close().await?;
                return Ok(());
            }
            ServerMessage::AuthError(error) => return Err(format!("{error:?}").into()),
            ServerMessage::VersionMismatch(error) => return Err(error.message.into()),
            ServerMessage::Error(error) => return Err(format!("{error:?}").into()),
            other => {
                println!("server: {other:?}");
            }
        }
    }

    Err("connection closed before authentication completed".into())
}
