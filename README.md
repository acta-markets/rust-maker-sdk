# acta-maker-sdk

Rust SDK for Acta's Solana options RFQ venue. It supports makers that receive
RFQs and send signed quotes, plus vault operators that quote, check served
transactions, and run capital operations. Features are opt-in; the default set is
empty.

## Install and credentials

Registration is not self-service. Acta must register the maker owner and its
quote-signing key on chain. Until then the connection fails with
`maker_not_registered`.

- The owner holds funds and signs maker transactions such as `DepositPremium`,
  `WithdrawPremium`, and `DepositFundsToPosition`.
- The registered quote-signing key authenticates and signs quotes. It can be the
  owner key.

Deposit a premium balance with `DepositPremium` before quoting. A fill debits its
premium from that balance.

Rust 1.89 or later is required.

```toml
[dependencies]
acta-maker-sdk = { version = "0.5.4", features = ["ws-client"] }
```

For registration and environment setup, use the
[Rust maker guide](https://beta.acta.markets/docs/quickstart/maker-rust-sdk).

## Quick start: managed maker

From this checkout, point the examples at a 64-byte Solana keypair JSON file:

```sh
export ACTA_KEYPAIR_PATH=/absolute/path/to/quote-signing-key.json
export ACTA_MAKER_OWNER=<owner base58>
cargo run --locked --features ws-client --example hello_auth
cargo run --locked --features ws-client,http --example managed_quote
```

`ACTA_WS_URL` defaults to wss://devnet-api.acta.markets. `DEVNET_WS`, `MAINNET_WS`,
`DEVNET_API`, and `MAINNET_API` name the public origins.

The quote examples fetch `GET /api/v1/venue` and call `.venue(&venue)`. Without
`http`, call `.welcome(&welcome)` after the session welcome. The examples send real
signed quotes at a fixed 1_000_000_000 price, so use a test maker on devnet.
They do not price, hedge, or fund.

The managed quote loop connects, authenticates, recovers state, subscribes to
RFQs, signs a quote, and sends it in the recovered epoch:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{maker::MakerQuoteClient, managed::*, types::*};
use std::{sync::Arc, time::Duration};
# async fn run(keypair: [u8; 64], owner: String, min_validity: Duration) -> Result<(), Box<dyn std::error::Error>> {
static NONCES: AtomicNonceGenerator = AtomicNonceGenerator::new();

let signer = Arc::new(BytesSigner::from_keypair(&keypair)?);
let maker_owner = decode_base58_32(&owner)?;
let config = ManagedWsConfig::new(
    DEVNET_WS,
    HelloData {
        protocol_version: WS_PROTOCOL_VERSION.to_string(),
        features: vec![],
        client_name: Some("my-bot".to_string()),
        client_version: None,
    },
    signer.clone(),
    owner,
)
.low_latency()
.with_initial_subscribe(SubscribeData {
    request_id: uuid::Uuid::new_v4(),
    channels: vec![WsChannel::Rfqs],
    underlying_mints: None,
    quote_mints: None,
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
`DepositFundsToPosition` (or fund_position) if required. Acta's keeper settles an
in-the-money funded position to the owner's ATA; no SDK settlement call is needed.

## Features and examples

| Feature | Enables |
|---|---|
| `ws-client` | `MakerQuoteClient`, `MakerDataClient`, `ManagedWsHandle`, and raw Tokio `WsClient` |
| `vault` | `VaultOperatorClient`, checked vault transaction signing, and projection feed; includes `ws-client` and `chain` |
| `chain` | Builders, derived addresses, checked account decoders, `VaultManager`, price routes, and served-transaction verification |
| `chain-rpc` | `ChainClient` over Solana RPC; extends `chain` |
| `http` | Read-only `HttpClient` using `reqwest` 0.13 and `rustls` with the platform verifier |
| `decimal` | Exact `rust_decimal` views for `Price`, `Strike`, and `Quantity` |
| `test-helpers` | `ManagedWsHandle::test_handle` and message injection |

| Example | Features | Purpose |
|---|---|---|
| `hello_auth` | `ws-client` | Authenticate, print success, close |
| `managed_quote` | `ws-client`, `http` | Recover state and send epoch-bound quotes |
| `quote` | `ws-client`, `http` | Raw `/maker` handshake and signed quote |
| `discovery` | `ws-client` | Read quotes, positions, markets, and summary on `/maker/data` |
| `indicative` | `ws-client` | Answer unsigned indicative-price requests |
| `loopback_latency` | `ws-client` | Local managed-client latency sample |
| `vault_operator` | `vault`, `http` | Vault session, checked fill and swap signing |
| `vault_capital` | `chain-rpc` | Capital-cycle crank |
| `curator_pt` | `chain-rpc`, `http` | PT route planning and simulation |

Maker examples read `ACTA_KEYPAIR_PATH`, `ACTA_MAKER_OWNER`, and `ACTA_WS_URL`.
`vault_operator` also needs `ACTA_VAULT_PDA`. `vault_capital` reads
`ACTA_KEYPAIR_PATH`, `ACTA_VAULT_PDA`, and `ACTA_RPC_URL` (devnet by default).

```sh
ACTA_VAULT_PDA=<vault> cargo run --locked --features vault,http --example vault_operator
ACTA_VAULT_PDA=<vault> ACTA_RPC_URL=<rpc> \
  cargo run --locked --features chain-rpc --example vault_capital
```

## Maker sessions and recovery

Use `MakerQuoteClient` on `/maker` for quotes, replacements, cancellations, and
subscriptions. Use `MakerDataClient` on `/maker/data` for private reads. The data
endpoint supplies `GetActiveRfqs`, `GetMyQuotes`, `GetOrderStatus`, `GetMakerPositions`,
`GetMarketsForMaker`, `GetMyTrades`, and `GetMmSummary`.

`MakerDataClient` also receives the owner's `PositionUpdated`, `TradeExecuted`, and
`MakerCapsUpdated` pushes after summary recovery. No subscription is needed for
those pushes. `all_maker_positions(filter, timeout)` pages `GetMakerPositions` when
`MmSummary.positions_has_more` is set.

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{managed::*, maker::MakerDataClient, types::*};
use std::sync::Arc;
# async fn run(hello: HelloData) -> Result<(), Box<dyn std::error::Error>> {
# let signer = Arc::new(BytesSigner::from_secret([1u8; 32]));
# let owner = signer.pubkey_base58();
let config = ManagedWsConfig::new("wss://devnet-api.acta.markets", hello, signer, owner)
    .with_endpoint(MakerWsEndpoint::Data);
let data = MakerDataClient::spawn(config)?;
data.wait_until_ready().await?;
let summary = data
    .request(
        GetMmSummaryMessage { request_id: uuid::Uuid::new_v4() },
        std::time::Duration::from_secs(2),
    )
    .await?;
# let _ = summary;
# Ok(())
# }
```

Keep a data session for monitoring, funding, or hot standby. On restart, use
`maker_open_positions(owner)` to recover open and funded positions from chain.
Open `/maker` only when taking over: a second quote session replaces the first and
may cancel its quotes if cancel-on-disconnect is enabled. Data pushes are not
replayed, so reconcile after a gap or reconnect. Up to four data sessions may
share one owner; a fifth gets `session_limit_exceeded`.

`Ready` means required SDK recovery reads arrived. It does not mean a strategy
applied them, and recovery is not one atomic snapshot. Apply recovery and
lifecycle messages before trading. Commands are tied to the current epoch;
`quote_in_epoch`, `replace_in_epoch`, and `batch_quotes_in_epoch` return `NotReady` once
the session advances.

Subscriptions survive resume and fresh authentication. The first local receiver
gets a bounded since-spawn backlog, so subscribe before waiting for traffic.
See the [maker API](https://beta.acta.markets/docs/reference/maker-api) and
[WS conventions](https://beta.acta.markets/docs/reference/ws-common) for delivery,
close codes, subscriptions, rate limits, and error variants.

### Queries, sends, and cancellation

`GetMyQuotes` with `Live` returns the complete current owner set without pagination.
`History` is persisted and paged. `GetOrderStatus` returns `Pending`, `Confirmed` with a
position address, or `Unknown`. An empty `Live` set, timeout, or `Unknown` does not
prove an interrupted order has no exposure.

send confirms a socket write. `send_await` waits for a correlated response, and
`try_send` returns a `SendTicket` whose drop before dequeue cancels that send. Quote,
replacement, batch, and cancellation messages share a FIFO lane; a full lane
returns `QueueFull`. `CancelQuoteAck` and `CancelAllQuotesAck` name the IDs actually
cancelled; a `QuoteCancelled` lifecycle event is not a cancellation receipt.

### Quote outcomes

Subscribe before sending. Create `QuoteOutcomeTracker::new(rfq_id, order_id, epoch, deadline)`.
Pass each `ManagedInbound` to `observe`, and call `check` with the
current epoch and monotonic time on transitions and at the deadline.

outcome reports `Pending`, `Acked`, `Selected`, terminal rejection, fill, expiry,
cancellation, loss, or `Unknown`. A selected order can remain unresolved after
`RfqClosed` until matching status proves a loss. A stream gap, closure, epoch
change, or deadline makes an unresolved order `Unknown`, never a proven failure.
Reconcile it through `GetMyQuotes` or `GetOrderStatus`; do not reuse its order ID.

## Quote construction and numeric units

`RfqBinding::from_broadcast` validates the market PDA, mints, expiry, and option
type under `ACTA_PROGRAM_ID` before signing. For another deployment, use
`from_broadcast_for_program` with a locally configured program ID. Never let an
RFQ choose the program ID.

`QuoteBuilder` requires `maker_owner`, `price`, validity bounds, and `nonce`. Read the
minimum validity and settlement buffer from `HttpClient::venue` or `WelcomeData`;
the builder rejects an impossible local validity window. Quote expiry is whole
Unix seconds. The full signing, validation, and replacement rules are in the
[Rust maker guide](https://beta.acta.markets/docs/quickstart/maker-rust-sdk) and
[maker API](https://beta.acta.markets/docs/reference/maker-api).

`Price` and `Strike` use the 1e9 `PRICE_SCALE` per underlying token, independent of
mint decimals. `Quantity` is underlying atoms. Without `decimal`, use
`Price::as_decimal`, `Strike::as_decimal`, `Quantity::to_units`, and
`total_premium_quote_atoms`. With `decimal`, use `Price::to_decimal`,
`Strike::to_decimal`, `Price::from_decimal_floor`, `Quantity::to_decimal`, and
`Quantity::from_decimal_floor`. The floor conversions return None for negative or
out-of-range values.

## Chain helpers and funding batches

The `chain` feature provides offline builders and checked account decoders.
With `chain-rpc`, `maker_snapshot` reads requested positions, premium ATAs, and
unique markets in batches of at most 100 keys. Missing positions and a missing
premium ATA are observations, not proof of closure or a failed RPC read.

Use `maker_open_positions(owner)` at startup or manual recovery, not in a tick
loop. `PositionSnapshot::fund_instructions` and `required_settlement` are pure.
For native settlement legs, prepend `wsol::build_wrap_sol_ixs` and aggregate the
deficit across the batch; budget the wallet balance, rent, and fees.
`NativeSolFunding::with_default_reserve` retains the single-position convenience.

`deposit_premium`, `withdraw_premium`, `fund_position`, and
`send_instructions_with_options` succeed only after confirmation at the client's
commitment. `send_instructions_with_blockhash` is submit-only and accepts one
shared blockhash for a batch.

With `chain-rpc` plus `ws-client`, call `confirm_signatures` once for the batch. It
uses `signatureSubscribe`, then one chunked status read for unresolved signatures
on a deadline or WebSocket failure. `signature_statuses` is available with
`chain-rpc` alone. `ConfirmationStatus::Pending` is unknown, not a reason to send a
new payment.

### Capital-cycle reads

For a vault crank, `ChainClient::unfunded_positions` reads held positions that
still need their settlement leg. `pending_capital_requests` reads queued deposits
and withdrawals from program accounts. `VaultManager` builds the capital-cycle
instructions, and `fund_maker_position` moves the required leg into a position the
vault holds. These are chain observations and builders; select a cadence and
authority policy in the operator.

## Vault owners

`chain::vault::VaultManager` builds bootstrap, stake top-up/unlock, governance
acceptance/revocation, recipient rotation, shutdown and close instructions. It also
builds typed updates and delegate rotation/disable. With `chain-rpc`,
`ChainClient::vault_manager(vault, actor)` loads the current configuration;
`*_from_chain` methods resolve proposal payers, recipients and required prices.
`close_vault_sequence()` creates governance destination ATAs before closing;
`unlock_manager_stake_sequence()` creates the recipient's share ATA before unlock.
The raw builders require those destination ATAs to exist. Rebuild the manager
after delegate, second-leg or governance transfer changes. Owner RPC paths check
governance and unconditional staging restrictions; permissionless apply/finalize
remain available. HWM reset cancellation is excluded from the owner RPC helper.

`build_settle_position_ix` derives settlement accounts from decoded position and
market snapshots for the wallet maker, including a vault writer; payout ATAs must
already exist. `settle_position_sequence` creates the used wallet destinations
for Funded positions, paid by the maker. It creates no ATAs for OTM Open and skips
vault-writer destinations. `ChainClient::settle_position_sequence` fetches the
snapshots; `unlock_manager_stake_sequence_from_chain` resolves the recipient.
These helpers build instructions; they do not submit transactions. See the
[vault lifecycle guide](https://beta.acta.markets/docs/protocol/vault-integration)
for contract rules.

## Vault operators

`VaultOperatorClient` authenticates an operator for one vault. The connection grant
is current-connection state; it disappears on disconnect or invalidation. Check
permissions and `Ready` before trading. The [vault operator
guide](https://beta.acta.markets/docs/protocol/vault-integration) owns the
role, lifecycle, funding, and protocol detail.

`verify_and_sign_sponsored_tx` and `verify_and_sign_swap` rebuild the transaction
from the operator's intent and sign only an exact match. Supply the vault account
read before the request, bounded compute budget, lookup tables, trade terms or
`PairPriceRoutes`, accepted staging routes, oracle shards, and an independent
minimum-output floor. `verify_vault_open` and `verify_vault_swap` expose the same
checks outside the client.

Checked signing verifies the fee payer, required signers, program IDs, account
privileges, instruction data, ATA setup, quote signature, price section, and
allowed swap hops. It checks deadlines but not the blockhash. A received ACK
only proves enqueueing; use `swap_status` or `SwapSettled` and refresh the vault for
execution state.

`VaultProjectionAdvanced` is a versioned projection cursor, not an execution
result. Ignore duplicates and older versions, then reload HTTP vault detail on a
newer version or a gap. `VaultProjection::follows` checks adjacency.

For a staged swap, request the same swap again only after the stage confirms.
The unstage must spend the whole staged amount at the open generation. A recovery
stage has its own `VaultSwapKind::RecoveryStage` shape; the checked signer rebuilds
it before signing.

## Price routes, venues, and staging

`PriceRoute` is the on-chain valuation source for a mint. For a served swap,
`PairPriceRoutes` supplies the vault's main and second routes, accepted staging
routes, and permitted push-oracle shards. The deployment's venue registry
controls the allowed hop programs. Read the detailed route, brick, and
checked-signing requirements in the [vault operator
guide](https://beta.acta.markets/docs/protocol/vault-integration).

For direct PT routing with `chain-rpc` and `http`, call `curator::Config::discover`
once, then use `Curator::plan` and `curator::execute`. Re-discover after a route
dependency or second-leg roll. A staged plan must refresh the slot and re-quote
before its unstage; `Plan::Staged::unstage` is a fit preview, not a transaction to
cache and send.

The curator_pt example simulates only; it never broadcasts a mainnet transaction:

```sh
RPC_URL=... JUPITER_API_KEY=... CURATOR_KEYPAIR=/path/to/keypair.json \
VAULT=... AMOUNT_ATOMS=1000000 \
cargo +stable run --example curator_pt --features chain-rpc,http -- buy
```

Use sell for PT input before maturity or redeem after maturity. A staged
simulation covers only the first half; the second needs confirmed staging state.

## HTTP, wire, and upgrades

`HttpClient` reads venue limits, tokens, markets, vaults, and vault positions.
Venue, token, and market responses use ETag revalidation; vault reads are not
cached. Read current limits from the service rather than hardcoding them.

Wire enum values are extensible. Match error codes, not messages, and include
fallback arms for non_exhaustive enums. *_unix_ms fields are milliseconds;
signed quote expiry is Unix seconds. Chain events are identified by
(signature, instruction_index).

Use `WS_PROTOCOL_VERSION` instead of hardcoding the protocol version.

Version 0.5.4 adds `WelcomeData` validity-window fields, the `decimal` feature,
maker-data owner pushes, `QuoteOutcomeTracker`, batch confirmation, `PriceRoute`
swaps, and staged recovery. See [CHANGELOG.md](CHANGELOG.md) for exact 0.5.4
and earlier migration notes, especially explicit maker owners and epoch-bound
quote methods.

## Latency sample

Release loopback sample with `low_latency` on one Apple silicon machine:

| Leg | p50 | p90 | p99 |
|---|---|---|---|
| `try_send` to peer frame | 20 µs | 27 µs | 60 µs |
| peer send to session parse | 150 µs | 281 µs | 424 µs |
| session task to subscriber recv | 19 µs | 39 µs | 70 µs |

This measures SDK overhead only; network and server time are additional.

## Documentation

- [Rust maker guide](https://beta.acta.markets/docs/quickstart/maker-rust-sdk)
- [Maker API reference](https://beta.acta.markets/docs/reference/maker-api)
- [WebSocket conventions](https://beta.acta.markets/docs/reference/ws-common)
- [Vault operator guide](https://beta.acta.markets/docs/protocol/vault-integration)
- [API reference](https://docs.rs/acta-maker-sdk)

## License

MIT
