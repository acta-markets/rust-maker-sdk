//! Subscribe to RFQs and submit signed quotes.

mod common;
#[path = "common/venue.rs"]
mod venue;

use acta_maker_sdk::ws::{client::WsClient, managed::normalize_maker_ws_url, types::*};
use acta_maker_sdk::{
    AtomicNonceGenerator, Nonce, Price, QuoteBuildError, QuoteExpiry, RfqBinding, SignerLike,
    WS_PROTOCOL_VERSION,
};
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let common::Config { signer, owner, url } = common::load()?;
    let owner = owner.ok_or("set ACTA_MAKER_OWNER to the registered maker owner")?;
    let maker_owner = acta_maker_sdk::decode_base58_32(&owner)?;
    let venue = venue::load(&url).await?;
    let lifetime = std::time::Duration::from_secs(u64::from(venue.min_quote_validity_seconds) + 60);
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
                let valid_until =
                    QuoteExpiry::after(lifetime).ok_or("system clock is before the Unix epoch")?;
                let quote = match RfqBinding::from_broadcast(&rfq)?
                    .quote()
                    .maker_owner(maker_owner)
                    .price(Price::new(1_000_000_000))
                    .valid_until(valid_until)
                    .venue(&venue)
                    .nonce(Nonce::new(NONCE_GEN.next_u64()?))
                    .sign(&signer)
                {
                    Ok(quote) => quote,
                    Err(QuoteBuildError::ValidUntilAfterMarketExpiry { .. }) => continue,
                    Err(error) => return Err(error.into()),
                };

                println!(
                    "submitting quote rfq={} strike={}",
                    quote.rfq_id, quote.strike
                );
                client.quote(quote).await?;
            }
            ServerMessage::AuthError(error) => return Err(format!("{error:?}").into()),
            ServerMessage::VersionMismatch(error) => return Err(error.message.into()),
            ServerMessage::Error(error) => return Err(format!("{error:?}").into()),
            other => println!("server: {other:?}"),
        }
    }

    Ok(())
}
