//! Quote call spreads, put spreads and cash puts. The gross per-unit price G must
//! satisfy 0 < G < width; the maker posts collateral C minus the net premium.

mod common;
#[path = "common/venue.rs"]
mod venue;

use acta_maker_sdk::spreads::{SpreadTerms, SpreadType};
use acta_maker_sdk::ws::{client::WsClient, managed::normalize_maker_ws_url, types::*};
use acta_maker_sdk::{
    AtomicNonceGenerator, Nonce, PositionType, Price, ProductSet, QuoteBuildError, QuoteExpiry,
    RfqBinding, SignerLike, WS_PROTOCOL_VERSION,
};
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

/// Placeholder pricing: a quarter of the width. Use your own model.
const PRICE_FRACTION_BPS: u64 = 2_500;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let common::Config { signer, owner, url } = common::load()?;
    let owner = owner.ok_or("set ACTA_MAKER_OWNER to the registered maker owner")?;
    let maker_owner = acta_maker_sdk::Address32::from_base58(&owner)?;
    let venue = venue::load(&url).await?;
    let tokens = venue::client(&url)?.tokens().await?;
    let lifetime = std::time::Duration::from_secs(u64::from(venue.min_quote_validity_seconds) + 60);
    let mut client = WsClient::connect(&normalize_maker_ws_url(&url)).await?;

    client
        .send_hello(HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            on_disconnect: OnDisconnect::KeepQuotes,
            client_name: "spread-bot".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .await?;

    while let Some(msg) = client.next().await {
        match msg? {
            ServerMessage::Welcome(_) => {
                client
                    .start_auth(acta_maker_sdk::Address32::from_base58(&owner)?)
                    .await?;
            }
            ServerMessage::AuthRequest(data) => {
                acta_maker_sdk::ws::auth::validate_maker_auth_challenge(&data.challenge)?;
                client
                    .auth_challenge(AuthChallengeData {
                        challenge: data.challenge.clone(),
                        signature: signer.sign_message_base58(data.challenge.as_bytes()),
                        pubkey: acta_maker_sdk::Address32::from_base58(&owner)?,
                    })
                    .await?;
            }
            ServerMessage::AuthSuccess(_) => {
                client
                    .subscribe(SubscribeData {
                        products: ProductSet::new(&[
                            PositionType::CallSpread,
                            PositionType::PutSpread,
                            PositionType::CashPut,
                        ])?,
                        request_id: Uuid::new_v4(),
                        channels: vec![WsChannel::Rfqs],
                        underlying_mints: vec![],
                        quote_mints: vec![],
                    })
                    .await?;
            }
            ServerMessage::RfqBroadcast(rfq) => {
                // A cash put executes as the put spread (0, K).
                let chain = rfq.terms.to_chain();
                let (Some(upper), Some(underlying), Some(quote_token)) = (
                    rfq.terms.upper_strike(),
                    tokens.get(&rfq.market.underlying_mint),
                    tokens.get(&rfq.market.quote_mint),
                ) else {
                    continue;
                };
                let terms = SpreadTerms {
                    kind: SpreadType::try_from(u8::from(chain.position_type()))?,
                    lower_strike: chain.strike().value(),
                    upper_strike: upper.value(),
                    quantity: rfq.quantity.value(),
                    quote_decimals: quote_token.decimals.value(),
                    underlying_decimals: underlying.decimals.value(),
                };
                let price = (terms.upper_strike - terms.lower_strike) * PRICE_FRACTION_BPS / 10_000;
                let premium = match terms.premium(
                    price,
                    venue.protocol_fee_bps_premium,
                    venue.protocol_fee_bps_volume,
                ) {
                    Ok(premium) => premium,
                    // Too small to price: G rounds to zero or reaches C.
                    Err(_) => continue,
                };
                let valid_until =
                    QuoteExpiry::after(lifetime).ok_or("system clock is before the Unix epoch")?;
                let quote =
                    match RfqBinding::from_broadcast_for_program(&rfq, venue.program_id.into())?
                        .quote()
                        .maker_owner(maker_owner)
                        .price(Price::new(price))
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
                    "quote rfq={} C={} gross={} fee={} net={} prefund={}",
                    quote.rfq_id,
                    premium.collateral,
                    premium.gross_premium,
                    premium.protocol_fee,
                    premium.net_premium,
                    premium.prefunded_quote,
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
