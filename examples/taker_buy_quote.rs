//! Write taker buys: subscribe to `TakerBuyRfqs`, keep Maker PDA custody funded and
//! quote asks. A buy's collateral leaves custody at open, so a short custody makes the
//! server skip the RFQ (`maker_insufficient_balance`) before it reaches you.

mod common;
#[path = "common/venue.rs"]
mod venue;

use acta_maker_sdk::chain::{
    ChainClient, MakerDepositArgs, derive_associated_token_address, maker_pda_with_program_id,
};
use acta_maker_sdk::ws::maker::MakerQuoteClient;
use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{
    AtomicNonceGenerator, Nonce, Price, QuoteBuildError, QuoteExpiry, RfqBinding,
    WS_PROTOCOL_VERSION,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer, read_keypair_file};
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

static NONCE_GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();

/// One custody mint held at a target: the underlying for calls, the quote mint for
/// puts and spreads.
struct Custody {
    chain: ChainClient,
    owner: Keypair,
    mint: Pubkey,
    token_program: Pubkey,
    target: u64,
}

impl Custody {
    fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let var = |name: &str| std::env::var(name).map_err(|_| format!("set {name}"));
        let rpc_url = std::env::var("ACTA_RPC_URL")
            .unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
        let program_id = Pubkey::new_from_array(acta_maker_sdk::ACTA_PROGRAM_ID);
        Ok(Self {
            chain: ChainClient::new(&rpc_url, program_id, CommitmentConfig::confirmed()),
            owner: read_keypair_file(var("ACTA_OWNER_KEYPAIR_PATH")?)?,
            mint: Pubkey::from_str(&var("ACTA_CUSTODY_MINT")?)?,
            token_program: Pubkey::from_str(&var("ACTA_CUSTODY_TOKEN_PROGRAM")?)?,
            target: var("ACTA_CUSTODY_TARGET")?.parse()?,
        })
    }

    /// Deposits from the owner's wallet ATA up to the target. Call it at start and
    /// after every fill; the deposit is a plain `MakerDeposit` and needs no server call.
    async fn top_up(&self) -> Result<(), Box<dyn std::error::Error>> {
        let (maker_pda, _) =
            maker_pda_with_program_id(&self.chain.program_id(), &self.owner.pubkey());
        let custody = derive_associated_token_address(&maker_pda, &self.mint, &self.token_program);
        let balance = self.chain.fetch_token_balance(&custody).await.unwrap_or(0);
        if balance >= self.target {
            return Ok(());
        }
        let signature = self
            .chain
            .maker_deposit(
                MakerDepositArgs {
                    maker_owner: self.owner.pubkey(),
                    amount: self.target - balance,
                    premium_mint: self.mint,
                    token_program: Some(self.token_program),
                    create_atas: true,
                },
                &self.owner,
                None,
            )
            .await?;
        tracing::info!(%signature, deposited = self.target - balance, "custody topped up");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let common::Config { signer, owner, url } = common::load()?;
    let owner = owner.ok_or("set ACTA_MAKER_OWNER to the registered maker owner")?;
    let maker_owner = acta_maker_sdk::Address32::from_base58(&owner)?;
    let custody = Custody::load()?;
    custody.top_up().await?;

    let venue = venue::load(&url).await?;
    let lifetime = std::time::Duration::from_secs(u64::from(venue.min_quote_validity_seconds) + 60);
    let signer = Arc::new(signer);
    let signer_for_quotes = Arc::clone(&signer);
    let config = ManagedWsConfig::new(
        url,
        HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            on_disconnect: OnDisconnect::KeepQuotes,
            client_name: "buy-writer".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        signer,
        maker_owner,
    )
    .low_latency()
    .with_initial_subscribe(SubscribeData {
        products: acta_maker_sdk::ProductSet::ALL,
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::TakerBuyRfqs],
        underlying_mints: vec![],
        quote_mints: vec![],
    });

    let client = MakerQuoteClient::spawn(config)?;
    let mut rx = client.subscribe_messages();
    client.wait_until_ready().await?;

    loop {
        let msg = match rx.recv().await {
            Ok(msg) => msg,
            Err(ManagedReceiveError::Closed) => break,
            Err(error) => return Err(format!("message stream failed: {error}").into()),
        };
        let epoch = msg.connection_epoch;
        match msg.message() {
            ServerMessage::RfqBroadcast(rfq) if rfq.side.is_taker_buy() => {
                let valid_until =
                    QuoteExpiry::after(lifetime).ok_or("system clock is before the Unix epoch")?;
                // On a buy the taker pays this price; a writer quotes an ask.
                let quote = match RfqBinding::from_broadcast_for_program(
                    rfq,
                    acta_maker_sdk::ACTA_PROGRAM_ID,
                )?
                .quote()
                .maker_owner(maker_owner)
                .price(Price::new(2_000_000_000))
                .valid_until(valid_until)
                .venue(&venue)
                .nonce(Nonce::new(NONCE_GEN.next_u64()?))
                .sign(signer_for_quotes.as_ref())
                {
                    Ok(quote) => quote,
                    Err(QuoteBuildError::ValidUntilAfterMarketExpiry { .. }) => continue,
                    Err(error) => return Err(error.into()),
                };
                match client.quote_in_epoch(quote, epoch).await {
                    Ok(()) | Err(ManagedWsError::NotReady) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            ServerMessage::RfqSkipped(skipped) => {
                tracing::warn!(rfq = %skipped.rfq_id, cap = ?skipped.cap_detail, "skipped: top up custody");
                custody.top_up().await?;
            }
            ServerMessage::QuoteFilled(fill) => {
                tracing::info!(rfq = %fill.rfq_id, "buy written; collateral left custody");
                custody.top_up().await?;
            }
            _ => {}
        }
    }
    Ok(())
}
