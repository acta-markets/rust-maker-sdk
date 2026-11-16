//! Simulates the first transaction of a mainnet PT buy/sell; never sends a transaction.
use acta_maker_sdk::chain::vault::{
    curator::{Config, Curator, Error, JupiterClient},
    router::Intent,
};
use solana_keypair::read_keypair_file;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let rpc = RpcClient::new(std::env::var("RPC_URL")?);
    let jupiter = JupiterClient::from_env()?;
    let signer = read_keypair_file(std::env::var("CURATOR_KEYPAIR")?).map_err(|e| e.to_string())?;
    let vault: Pubkey = std::env::var("VAULT")?.parse()?;
    let program = Pubkey::new_from_array(acta_maker_sdk::ACTA_PROGRAM_ID);
    // Mainnet Exponent PT market, SY and core vault for the route.
    let config = Config::discover(
        &rpc,
        program,
        vault,
        signer.pubkey(),
        solana_pubkey::pubkey!("8JCVSxVrM1f5stBwNWko2D5Yt9p4wJQsvPVWFZBkzhmo"),
        solana_pubkey::pubkey!("9QReDA3ndt8FBQVJHGzM8h6NKojqyh2Ej8cqFn9dY6Hn"),
        solana_pubkey::pubkey!("Ciz4yqREYbMH4dtQvB3Rc2mSVEGMs8jFfE97qj6nXAPu"),
        0,
    )
    .await?;
    let curator = Curator::new(&rpc, &jupiter, &signer, config, 50, 60)?;
    let amount = std::env::var("AMOUNT_ATOMS")?.parse()?;
    let intent = match std::env::args().nth(1).as_deref() {
        Some("buy") => Intent::Buy(amount),
        Some("sell") => Intent::Sell(amount),
        Some("redeem") => Intent::Redeem(amount),
        _ => return Err("usage: curator_pt buy|sell|redeem".into()),
    };
    curator.simulate(intent).await?;
    println!("First transaction simulated successfully; nothing sent.");
    Ok(())
}
