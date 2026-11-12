//! Read a maker's own state on `/maker/data`: custody balances, what to reconcile after a
//! reconnect, and how settled positions paid out.
//!
//! - Custody: `MyCaps.balances` has, per mint, what sits in Maker PDA custody (`deposited`),
//!   what open positions and live buy quotes hold (`committed`) and what is left (`available`).
//!   Top up with a plain `MakerDeposit`, as in `taker_buy_quote`.
//! - Reconnect: a quote that was being filled when the socket dropped is `awaiting_signature`
//!   or `executing` in the live book; ask `GetOrderStatus` for each before quoting that RFQ
//!   again. `unknown` is not proof that the order did not execute.
//! - Settlement: a settled position carries its fixing and the atoms each side received in
//!   `PositionSettlement`.

mod common;

use std::time::Duration;

use acta_maker_sdk::ws::maker::MakerDataClient;
use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{Address32, WS_PROTOCOL_VERSION};
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let common::Config { signer, owner, url } = common::load()?;
    let owner = owner.ok_or("set ACTA_MAKER_OWNER to the registered maker owner")?;
    let config = ManagedWsConfig::new(
        url,
        HelloData {
            protocol_version: WS_PROTOCOL_VERSION.to_string(),
            on_disconnect: OnDisconnect::KeepQuotes,
            client_name: "maker-state".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        std::sync::Arc::new(signer),
        Address32::from_base58(&owner)?,
    )
    .with_endpoint(MakerWsEndpoint::Data);
    let data = MakerDataClient::spawn(config)?;
    data.wait_until_ready().await?;

    let caps = data
        .request_all(
            GetMyCapsMessage {
                limit: 200,
                cursor: Cursor::Start {},
                request_id: Uuid::new_v4(),
            },
            TIMEOUT,
        )
        .await?
        .into_payload();
    for balance in &caps.balances {
        println!(
            "custody {}: deposited {} committed {} available {}",
            balance.symbol, balance.deposited, balance.committed, balance.available
        );
    }

    let quotes = data
        .request_all(
            GetMyQuotesMessage {
                limit: 200,
                cursor: Cursor::Start {},
                request_id: Uuid::new_v4(),
                scope: MakerQuoteScope::Live {},
            },
            TIMEOUT,
        )
        .await?
        .into_payload();
    for quote in &quotes.quotes {
        if !matches!(
            quote.state,
            MakerQuoteState::AwaitingSignature { .. } | MakerQuoteState::Executing { .. }
        ) {
            continue;
        }
        let status = data
            .request_all(
                GetOrderStatusMessage {
                    request_id: Uuid::new_v4(),
                    order_id: quote.order_id,
                },
                TIMEOUT,
            )
            .await?
            .into_payload();
        match status.state {
            OrderExecutionState::Confirmed { position_pda } => {
                println!("rfq {}: filled, position {position_pda}", quote.rfq_id);
            }
            OrderExecutionState::Pending => println!("rfq {}: fill in flight", quote.rfq_id),
            OrderExecutionState::Unknown => {
                println!("rfq {}: unknown, do not requote it yet", quote.rfq_id);
            }
        }
    }

    let settled = data
        .request_all(
            GetMakerPositionsMessage {
                request_id: Uuid::new_v4(),
                market_pda: QueryFilter::All {},
                underlying_mint: QueryFilter::All {},
                status: vec![PositionStatus::Settled],
                min_expiry_ts: TimeBound::Unbounded {},
                limit: 200,
                cursor: Cursor::Start {},
            },
            TIMEOUT,
        )
        .await?
        .into_payload();
    for position in &settled.positions {
        let PositionState::Settled { settlement } = &position.state else {
            continue;
        };
        match settlement {
            PositionSettlement::Physical {
                settlement_price,
                is_otm,
                holder,
                writer,
            } => println!(
                "{}: physical at {settlement_price} (otm {is_otm}); holder got {} collateral and {} settlement atoms, writer {} and {}",
                position.pda,
                holder.collateral_atoms,
                holder.settlement_atoms,
                writer.collateral_atoms,
                writer.settlement_atoms,
            ),
            PositionSettlement::NotExercised { settlement_price } => println!(
                "{}: buyer did not deposit; collateral returned at {settlement_price}",
                position.pda
            ),
            PositionSettlement::Cash {
                settlement_price,
                holder_payoff,
                writer_return,
            } => println!(
                "{}: cash at {settlement_price}, holder {holder_payoff}, writer {writer_return}",
                position.pda
            ),
        }
    }

    data.close().await?;
    Ok(())
}
