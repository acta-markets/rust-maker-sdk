//! Operate a vault over `/vault`: quote, sign sponsored fills, follow the projection.
//! `_unverified` signing trusts the server; production should use the checked methods.

#[allow(dead_code)]
mod common;

use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{
    AtomicNonceGenerator, Nonce, Price, QuoteExpiry, RfqBinding, VaultOperatorClient,
    VaultOperatorConfig, WS_PROTOCOL_VERSION,
};
use std::sync::Arc;
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let common::Config { signer, url, .. } = common::load()?;
    let operator = Arc::new(signer);
    let vault_pda = std::env::var("ACTA_VAULT_PDA")
        .map_err(|_| "set ACTA_VAULT_PDA to the vault this operator key is delegated for")?;

    let config = VaultOperatorConfig::new(
        url,
        HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            features: vec![],
            client_name: Some("vault-operator".to_string()),
            client_version: Some("0.1.0".to_string()),
        },
        vault_pda,
        operator,
    )
    .with_managed(|managed| managed.low_latency());

    let client = VaultOperatorClient::spawn(config)?;
    let mut messages = client.subscribe_messages();

    let mut projections = client.subscribe_projections();
    tokio::spawn(async move {
        let mut last_version = None;
        loop {
            let step = match projections.recv().await {
                Ok(step) => step,
                Err(ManagedReceiveError::Gap { skipped }) => {
                    tracing::warn!(skipped, "reload vault HTTP detail");
                    continue;
                }
                Err(_) => break,
            };
            if last_version.is_some_and(|previous| step.version() <= previous) {
                continue;
            }
            if let Some(previous) = last_version
                && !step.follows(previous)
            {
                tracing::warn!(previous, current = step.version(), "projection gap; reload");
            }
            tracing::info!(version = step.version(), "reload vault HTTP detail");
            last_version = Some(step.version());
        }
    });

    client.wait_until_ready().await?;
    let auth = client.auth().ok_or("no vault auth after ready")?;
    tracing::info!(
        operator_kind = ?auth.operator_kind,
        can_maker = client.can_maker(),
        can_taker = client.can_taker(),
        "vault session ready"
    );

    loop {
        let msg = match messages.recv().await {
            Ok(msg) => msg,
            Err(ManagedReceiveError::Gap { skipped }) => {
                tracing::error!(skipped, "message stream lagged; refresh state");
                continue;
            }
            Err(ManagedReceiveError::Closed) => break,
            Err(error) => return Err(format!("message stream failed: {error}").into()),
        };
        match msg.message() {
            // Maker side: the vault is the maker of record, the delegate signs.
            ServerMessage::RfqBroadcast(rfq) if client.can_maker() => {
                let valid_until = QuoteExpiry::after(std::time::Duration::from_secs(350))
                    .ok_or("system clock is before the Unix epoch")?;
                if valid_until
                    .to_system_time()
                    .ok_or("quote expiry exceeds SystemTime")?
                    > rfq.market.expiry_ts
                {
                    continue;
                }
                let binding = RfqBinding::from_broadcast(rfq)?;
                let quote = client.sign_quote(
                    &binding
                        .quote()
                        .price(Price::new(1_000_000_000))
                        .valid_until(valid_until)
                        .nonce(Nonce::new(NONCE_GEN.next_u64()?)),
                )?;
                client.quote(quote).await?;
            }
            // Taker side: the server builds the sponsored fill, the operator signs it.
            ServerMessage::SponsoredTxToSign(to_sign) => {
                let signed = client.sign_sponsored_tx_unverified(to_sign)?;
                client.submit_signed_sponsored_tx(signed).await?;
            }
            ServerMessage::OrderConfirmed(confirmed) => {
                tracing::info!(position = %confirmed.position_pda, "vault position opened");
            }
            ServerMessage::QuoteFilled(fill) => {
                tracing::info!(rfq = %fill.rfq_id, position = %fill.position_pda, "filled");
            }
            ServerMessage::Error(err) => tracing::error!(?err, "session error"),
            _ => {}
        }
    }

    Ok(())
}

#[allow(dead_code)]
async fn rebalance(
    client: &VaultOperatorClient,
    input_mint: &str,
    output_mint: &str,
    amount: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let quote = client
        .swap_quote(SwapQuoteData {
            request_id: Uuid::new_v4(),
            vault_pda: client.vault_pda().to_string(),
            mode: VaultSwapMode::Rebalance,
            input_mint: input_mint.to_string(),
            output_mint: output_mint.to_string(),
            amount: amount.to_string(),
            slippage_bps: 50,
        })
        .await?;
    let submit = client.sign_swap_unverified(&quote)?;
    let ack = client.swap_submit(submit).await?;
    tracing::info!(execution = ?ack.execution_id, "swap queued");
    Ok(())
}
