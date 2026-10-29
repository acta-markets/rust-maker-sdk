mod common;

use acta_maker_sdk::SignerLike;
use acta_maker_sdk::ws::{client::WsClient, managed::normalize_maker_data_ws_url, types::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let common::Config { signer, owner, url } = common::load()?;
    let owner = owner.ok_or("set ACTA_MAKER_OWNER to the registered maker owner")?;
    let mut client = WsClient::connect(&normalize_maker_data_ws_url(&url)).await?;

    let hello = HelloData {
        protocol_version: acta_maker_sdk::WS_PROTOCOL_VERSION.to_string(),
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
                let auth = AuthChallengeData {
                    signature: signer.sign_message_base58(data.challenge.as_bytes()),
                    challenge: data.challenge,
                    pubkey: acta_maker_sdk::Address32::from_base58(&owner.clone())?,
                };
                client.auth_challenge(auth).await?;
            }
            ServerMessage::AuthSuccess(_) => {
                client
                    .get_my_quotes(GetMyQuotesMessage {
                        request_id: uuid::Uuid::new_v4(),
                        scope: MakerQuoteScope::Live {},
                    })
                    .await?;

                client
                    .get_maker_positions(GetMakerPositionsMessage {
                        request_id: uuid::Uuid::new_v4(),
                        market: QueryFilter::All {},
                        underlying_mint: QueryFilter::All {},
                        status: vec![PositionStatus::Open, PositionStatus::Funded],
                        min_expiry_ts: TimeBound::Unbounded {},
                        limit: 100,
                        cursor: PageCursor::Start {},
                    })
                    .await?;

                client
                    .get_markets_for_maker(GetMarketsForMakerMessage {
                        request_id: uuid::Uuid::new_v4(),
                        underlying_mints: vec![],
                        quote_mints: vec![],
                        min_expiry_ts: TimeBound::Unbounded {},
                        max_expiry_ts: TimeBound::Unbounded {},
                        is_put: QueryFilter::All {},
                    })
                    .await?;

                client.get_mm_summary().await?;
            }
            ServerMessage::AuthError(error) => return Err(format!("{error:?}").into()),
            ServerMessage::VersionMismatch(error) => return Err(error.message.into()),
            ServerMessage::Error(error) => return Err(format!("{error:?}").into()),
            other => println!("server: {other:?}"),
        }
    }

    Ok(())
}
