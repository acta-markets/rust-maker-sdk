# acta-maker-sdk

Rust SDK for Acta's Solana options RFQ venue. Makers use it to receive RFQs and
send signed quotes. Vault operators use it to quote for a vault, check served
transactions before signing, and run capital operations.

## Install

```toml
[dependencies]
acta-maker-sdk = { version = "0.7.0", features = ["ws-client"] }
```

Rust 1.89 or later. The default feature set is empty.

| Feature | Enables |
|---|---|
| `ws-client` | `MakerQuoteClient`, `MakerDataClient`, `ManagedWsHandle` and the raw Tokio `WsClient` |
| `http` | Read-only `HttpClient` (`reqwest` 0.13 with `rustls`) |
| `chain` | Instruction builders, derived addresses, checked account decoders, `VaultManager`, served-transaction verification |
| `chain-rpc` | `ChainClient` over Solana RPC; includes `chain` |
| `vault` | `VaultOperatorClient` and checked vault transaction signing; includes `ws-client` and `chain` |
| `decimal` | `rust_decimal` conversions for `Price`, `Strike` and `Quantity` |
| `test-helpers` | `ManagedWsHandle::test_handle` and message injection |

This release speaks WS protocol `1.0.0` (`WS_PROTOCOL_VERSION`). See
[CHANGELOG.md](CHANGELOG.md) for what changed.

## Before you connect

Maker registration is not self-service. Acta registers the maker owner and its
quote-signing key on chain; until then authentication fails with
`maker_not_registered`.

- The owner holds funds and signs maker transactions such as `MakerDeposit`
  and `DepositFundsToPosition`.
- The quote-signing key authenticates the session and signs quotes. It can be
  the owner key.
- A fill debits its premium from the owner's premium balance, so deposit with
  `MakerDeposit` before quoting.

The [Rust maker guide](https://beta.acta.markets/docs/quickstart/maker-rust-sdk)
covers registration and environment setup.

## Quick start

The examples read a 64-byte Solana keypair JSON file:

```sh
export ACTA_KEYPAIR_PATH=/absolute/path/to/quote-signing-key.json
export ACTA_MAKER_OWNER=<owner base58>
cargo run --locked --features ws-client --example hello_auth
cargo run --locked --features ws-client,http --example managed_quote
```

`ACTA_WS_URL` defaults to `DEVNET_WS`. The quote examples send real signed
quotes at a fixed price of 1_000_000_000, so use a test maker on devnet. They do
not price, hedge or fund.

A managed quote loop connects, authenticates, recovers state, subscribes to RFQs
and quotes in the recovered epoch:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{maker::MakerQuoteClient, managed::*, types::*};
use std::{sync::Arc, time::Duration};
# async fn run(keypair: [u8; 64], owner: String, min_validity: Duration) -> Result<(), Box<dyn std::error::Error>> {
static NONCES: AtomicNonceGenerator = AtomicNonceGenerator::new();

let signer = Arc::new(BytesSigner::from_keypair(&keypair)?);
let maker_owner = Address32::from_base58(&owner)?;
let config = ManagedWsConfig::new(
    DEVNET_WS,
    HelloData {
        protocol_version: WS_PROTOCOL_VERSION.to_string(),
        on_disconnect: OnDisconnect::CancelQuotes,
        client_name: "my-bot".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    },
    signer.clone(),
    maker_owner,
)
.low_latency()
.with_initial_subscribe(SubscribeData {
    products: ProductSet::ALL,
    request_id: uuid::Uuid::new_v4(),
    channels: vec![WsChannel::Rfqs],
    underlying_mints: Vec::new(),
    quote_mints: Vec::new(),
});

let client = MakerQuoteClient::spawn(config)?;
let mut messages = client.subscribe_messages();
let epoch = client.wait_until_ready().await?;

while let Ok(msg) = messages.recv().await {
    match msg.message() {
        ServerMessage::RfqBroadcast(rfq) => {
            let min_valid_until = QuoteExpiry::after(min_validity).ok_or("clock")?;
            let valid_until =
                QuoteExpiry::after(min_validity + Duration::from_secs(60)).ok_or("clock")?;
            let quote = match RfqBinding::from_broadcast(rfq)?
                .quote()
                .maker_owner(maker_owner)
                .price(Price::new(1_000_000_000))
                .valid_until(valid_until)
                .min_valid_until(min_valid_until)
                .nonce(Nonce::new(NONCES.next_u64()?))
                .sign(signer.as_ref())
            {
                Ok(quote) => quote,
                Err(QuoteBuildError::ValidUntilAfterMarketExpiry { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            client.quote_in_epoch(quote, epoch).await?;
        }
        ServerMessage::QuoteAcknowledged(ack) => println!("accepted {:?}", ack.order_id),
        ServerMessage::QuoteOutbid(outbid) => println!("outbid on {}", outbid.rfq_id),
        ServerMessage::QuoteRejected(rejected) => println!("rejected {:?}", rejected.reason),
        ServerMessage::QuoteFilled(fill) => {
            println!("filled on {}, position {}", fill.market_pda, fill.position_pda)
        }
        ServerMessage::RfqClosed(closed) => println!("closed {:?}", closed.reason),
        _ => {}
    }
}
# Ok(())
# }
```

This sketch quotes only in its first epoch. `managed_quote` waits for each new
epoch and applies recovery before it quotes again. A socket write is not an
accepted quote; never replay an old signed quote after a reconnect.

`QuoteFilled` means the position opened on chain. Fund its settlement leg with
`DepositFundsToPosition` if the product requires it. Acta's keeper settles an
in-the-money funded position to the owner's ATA; no SDK call is needed.

## Examples

| Example | Features | Purpose |
|---|---|---|
| `hello_auth` | `ws-client` | Authenticate and close |
| `managed_quote` | `ws-client`, `http` | Recover state and send epoch-bound quotes |
| `taker_buy_quote` | `ws-client`, `http`, `chain-rpc` | Write taker buys from Maker PDA custody and keep it funded |
| `quote` | `ws-client`, `http` | Raw `/maker` handshake and one signed quote |
| `spread_quote` | `ws-client`, `http` | Price and sign call spreads, put spreads and cash puts |
| `discovery` | `ws-client` | Read quotes, positions, markets and summary on `/maker/data` |
| `indicative` | `ws-client` | Answer unsigned indicative-price requests |
| `loopback_latency` | `ws-client` | Measure local managed-client latency |
| `vault_operator` | `vault`, `http` | Vault session with checked fill and swap signing |
| `vault_capital` | `chain-rpc` | Capital-cycle crank |
| `curator_pt` | `chain-rpc`, `http` | Plan a PT route and simulate it; never sends |

Maker examples read `ACTA_KEYPAIR_PATH`, `ACTA_MAKER_OWNER` and `ACTA_WS_URL`.
`taker_buy_quote` also reads `ACTA_OWNER_KEYPAIR_PATH`, `ACTA_CUSTODY_MINT`,
`ACTA_CUSTODY_TOKEN_PROGRAM`, `ACTA_CUSTODY_TARGET` (atoms) and `ACTA_RPC_URL`.
`vault_operator` also reads `ACTA_VAULT_PDA`. `vault_capital` reads
`ACTA_KEYPAIR_PATH`, `ACTA_VAULT_PDA` and `ACTA_RPC_URL` (devnet by default).
`curator_pt` reads `RPC_URL`, `JUPITER_API_KEY`, `CURATOR_KEYPAIR`, `VAULT` and
`AMOUNT_ATOMS`, and takes `buy`, `sell` or `redeem` as its argument.

## Writing taker buys

A buy RFQ (`side: buy`) asks you to write the option the taker holds: the taker is
the holder, you are the writer. Subscribe to `WsChannel::TakerBuyRfqs`; the channel
is separate from `Rfqs`, so a maker that only sells never receives buys.

- **Custody.** The collateral leaves your Maker PDA custody at open, not your
  wallet: the underlying for a call, the quote mint for a put, a spread or a cash
  put. The net premium lands in custody in the same transaction. Fund custody with
  `ChainClient::maker_deposit` (`MakerDeposit`) from your wallet ATA; a wSOL
  underlying must already be wrapped in that ATA.
- **Admission.** The server checks custody before it sends you the RFQ. A short
  custody turns into `RfqSkipped` with `maker_insufficient_balance` and the RFQ never
  reaches quoting, so top up when you see the skip, after every fill, or on a timer.
- **Price.** The taker pays your price; quote an ask. The order id signs the side,
  so a buy quote cannot fill a sell.
- **Settlement.** The holder deposits strictly before expiry to exercise a physical
  buy. An unfunded in-the-money buy returns all collateral to you and settles as
  `not_exercised`; spreads and cash puts settle in cash with no deposit. `PositionInfo`
  names the roles `writer` (you) and `holder` (the buyer).

See [`examples/taker_buy_quote.rs`](examples/taker_buy_quote.rs).

## Rules that matter

- **Epochs.** `Ready` means the SDK's recovery reads arrived, not that your
  strategy applied them. `quote_in_epoch`, `replace_in_epoch` and
  `batch_quotes_in_epoch` return `NotReady` once the session moves to a new epoch.
- **Subscribe first.** The first receiver gets a bounded backlog since spawn;
  later receivers start at subscription time.
- **Unknown is not failure.** A timeout, gap, reconnect or `Unknown` order status
  does not prove an order has no exposure. Reconcile with `GetMyQuotes` or
  `GetOrderStatus` and never reuse its order ID.
- **Program ID.** `RfqBinding::from_broadcast` checks the market against
  `ACTA_PROGRAM_ID`. For another deployment use `from_broadcast_for_program` with a
  locally configured ID; never take it from the RFQ.
- **Units.** `Price` and `Strike` use the 1e9 `PRICE_SCALE` per underlying token.
  `Quantity` is underlying atoms. Signed quote expiry is whole Unix seconds;
  `*_unix_ms` fields are milliseconds.
- **Closed wire.** Every field is present on every frame and absent facts are
  `null`. Decoding rejects unknown message types, enum values, error codes and
  missing fields. A rejected frame after `Ready` keeps the session up and reaches
  you as `ManagedWsEvent::UnreadableFrame` and a receiver gap: reconcile as after
  any gap. Match error codes, not messages.
- **Side.** Every RFQ, quote, position and trade carries `terms: OptionTerms` and
  `side`. Makers subscribe to `WsChannel::TakerBuyRfqs` to receive `buy` RFQs, physical
  or spread, and quote them from custody. Vaults stay `sell` only; the builders
  reject those buys with `taker_buy_unsupported`.

## Where to start in the API

| Task | Start with |
|---|---|
| Quote, replace, cancel | `MakerQuoteClient`, `RfqBinding`, `QuoteBuilder` |
| Private reads and owner pushes | `MakerDataClient` (`/maker/data`), `all_pages` |
| Track one quote to a result | `ws::maker::QuoteOutcomeTracker` |
| Spreads and cash puts | `OptionTerms`, `spreads::SpreadTerms`, `build_spread_order_preimage` |
| Premium and fee split | `physical_premium`, `SpreadTerms::premium` |
| Recover positions from chain | `ChainClient::maker_open_positions`, `maker_snapshot` |
| Fund and confirm | `PositionSnapshot::fund_instructions`, `confirm_signatures` |
| Vault owner actions | `chain::vault::VaultManager`, `ChainClient::vault_manager` |
| Vault operator session | `VaultOperatorClient`, `verify_and_sign_sponsored_tx`, `verify_and_sign_swap` |
| Vault capital cycle | `ChainClient::unfunded_positions`, `pending_capital_requests` |
| PT routing | `chain::vault::curator` |
| Venue limits, tokens, markets, vaults | `HttpClient` |

Read venue limits such as minimum quote validity from `HttpClient::venue` or
`WelcomeData` rather than hardcoding them.

## Latency sample

Release build, loopback peer, `low_latency`, one Apple silicon machine. This is
SDK overhead only; network and server time are extra.

| Leg | p50 | p90 | p99 |
|---|---|---|---|
| `try_send` to peer frame | 20 µs | 27 µs | 60 µs |
| peer send to session parse | 150 µs | 281 µs | 424 µs |
| session task to subscriber recv | 19 µs | 39 µs | 70 µs |

## List paging

List calls collect every page for you. `MakerDataClient::request_all` and the
`HttpClient` catalog methods go through `paging::fetch_all`. For a "load more" view
use `request_page` and keep the returned cursor as an opaque value.

On the wire, WS lists take `limit` (at most 200) and `Cursor::Start {}` or
`Cursor::After { id }`, and return `next_cursor`, `null` on the last page. REST lists
take an optional `limit` (default 50, at most 200, 100 for vault requests) and an
optional `cursor`: omit it for the first page, then pass `next_cursor` as is.

## Documentation

- [Rust maker guide](https://beta.acta.markets/docs/quickstart/maker-rust-sdk)
- [Maker API reference](https://beta.acta.markets/docs/reference/maker-api)
- [WebSocket conventions](https://beta.acta.markets/docs/reference/ws-common)
- [HTTP API](https://beta.acta.markets/docs/reference/http-api)
- [Vault API, SDKs and swaps](https://beta.acta.markets/docs/protocol/vault-integration)
- [API reference on docs.rs](https://docs.rs/acta-maker-sdk)

## License

MIT
