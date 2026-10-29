//! Operate a vault over `/vault`: quote, follow the projection, sign fills and swaps only after
//! verifying them against locally known terms (`sign_fill`, `rebalance`).

#[allow(dead_code)]
mod common;
#[path = "common/venue.rs"]
mod venue;

use acta_maker_sdk::chain::ChainClient;
use acta_maker_sdk::chain::vault::{
    ComputeBudgetBounds, PairPriceRoutes, VaultAccount, VaultManager, VaultOpenIntent,
    VaultSaleTerms, VaultSwapIntent,
};
use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{
    AtomicNonceGenerator, Nonce, Price, QuoteBuildError, QuoteExpiry, RfqBinding, Ungated,
    VaultOperatorClient, VaultOperatorConfig, WS_PROTOCOL_VERSION,
};
use solana_sdk::message::AddressLookupTableAccount;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let common::Config { signer, url, .. } = common::load()?;
    let operator = Arc::new(signer);
    let venue = venue::load(&url).await?;
    let lifetime = std::time::Duration::from_secs(u64::from(venue.min_quote_validity_seconds) + 60);
    let vault_pda = std::env::var("ACTA_VAULT_PDA")
        .map_err(|_| "set ACTA_VAULT_PDA to the vault this operator key is delegated for")?;

    let config = VaultOperatorConfig::new(
        acta_maker_sdk::ProductSet::ALL,
        url,
        HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            on_disconnect: acta_maker_sdk::ws::types::OnDisconnect::KeepQuotes,
            client_name: "vault-operator".to_string(),
            client_version: "0.1.0".to_string(),
        },
        acta_maker_sdk::Address32::from_base58(&vault_pda)?,
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
            ServerMessage::RfqBroadcast(rfq) if client.can_maker() && !rfq.terms.is_spread() => {
                let valid_until =
                    QuoteExpiry::after(lifetime).ok_or("system clock is before the Unix epoch")?;
                let binding = RfqBinding::from_broadcast(rfq)?;
                let quote = match client.sign_quote(
                    &binding
                        .quote()
                        .price(Price::new(1_000_000_000))
                        .valid_until(valid_until)
                        .venue(&venue)
                        .nonce(Nonce::new(NONCE_GEN.next_u64()?)),
                ) {
                    Ok(quote) => quote,
                    Err(QuoteBuildError::ValidUntilAfterMarketExpiry { .. }) => continue,
                    Err(error) => return Err(error.into()),
                };
                match client.quote_in_epoch(quote, msg.connection_epoch).await {
                    Ok(()) | Err(ManagedWsError::NotReady) => {}
                    Err(error) => return Err(error.into()),
                }
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

const COMPUTE_BOUNDS: ComputeBudgetBounds = ComputeBudgetBounds {
    max_unit_limit: 600_000,
    max_unit_price_micro_lamports: 50_000,
};

#[allow(dead_code)]
async fn sign_fill(
    client: &VaultOperatorClient,
    chain: &ChainClient,
    holder: &VaultManager,
    taker: Pubkey,
    terms: VaultSaleTerms,
    lookup_tables: &[AddressLookupTableAccount],
    to_sign: &SponsoredTxToSignData,
) -> Result<(), Box<dyn std::error::Error>> {
    let keeper = chain.hot_authority().await?;
    let signed = client.verify_and_sign_sponsored_tx(
        to_sign,
        &VaultOpenIntent::Fill {
            keeper,
            holder,
            taker,
            terms,
        },
        lookup_tables,
        COMPUTE_BOUNDS,
    )?;
    client.submit_signed_sponsored_tx(signed).await?;
    Ok(())
}

#[allow(dead_code, clippy::too_many_arguments)]
async fn rebalance(
    client: &VaultOperatorClient,
    vault: &VaultManager,
    price_routes: PairPriceRoutes<'_>,
    lookup_tables: &[AddressLookupTableAccount],
    direction: VaultSwapDirection,
    amount: &str,
    min_output_floor: u64,
    // Read fresh: the second leg and the staging slot are checked against it.
    account: &VaultAccount,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = SwapQuoteData {
        request_id: Uuid::new_v4(),
        vault_pda: *client.vault_pda(),
        mode: VaultSwapMode::Rebalance,
        direction,
        amount: amount.to_string(),
        slippage_bps: 50,
    };
    let quote = client.swap_quote(request.clone()).await?;
    let submit = client.verify_and_sign_swap(
        &VaultSwapIntent {
            vault,
            request: &request,
            quote: &quote,
            price_routes,
            min_output_floor,
            account: *account,
        },
        lookup_tables,
        COMPUTE_BOUNDS,
    )?;
    let epoch = client.wait_until_ready().await?;
    let ack = client.swap_submit_in_epoch(submit, epoch, &Ungated).await?;
    tracing::info!(execution = ?ack.execution_id, "swap queued");
    Ok(())
}
