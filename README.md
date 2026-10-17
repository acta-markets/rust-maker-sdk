# acta-maker-sdk

Rust SDK for the Acta options RFQ venue on Solana. It is for two kinds of users:
**makers**, who receive RFQs over WebSocket and answer with signed quotes, and
**vault operators**, who quote, sign server-built transactions and crank the
capital cycle for an Acta vault. It covers authentication, managed WebSocket
sessions with reconnect and recovery, order signing, Solana instruction builders
and a read-only HTTP client. All features are opt-in; defaults are empty.

## Quickstart (maker)

### 1. Get registered

There is no self-service registration. Request access through the
[integration documentation](https://beta.acta.markets/docs); Acta registers your
maker on chain with the admin-gated, timelocked `RegisterMaker` instruction.
Until then every connection ends with `maker_not_registered`.

You need two keys (they may be the same key):

- **Owner**: the registered maker identity. It holds funds and signs on-chain
  maker transactions (`DepositPremium`, `WithdrawPremium`, `FundPosition`).
- **Quote-signing key**: registered together with the owner. It signs the
  authentication challenge and every quote; both name the owner as the maker.

Before quoting, deposit a premium balance with `DepositPremium`: the premium of
a fill is debited from it.

### 2. Install

Requires Rust 1.89 or later.

```toml
[dependencies]
acta-maker-sdk = { version = "0.4.4", features = ["ws-client"] }
```

### 3. Run the examples

From a checkout of this repository, point the examples at the quote-signing key
as a standard 64-byte Solana keypair JSON file:

```sh
export ACTA_KEYPAIR_PATH=/absolute/path/to/quote-signing-key.json
export ACTA_MAKER_OWNER=<owner base58>   # omit if the signing key is the owner
cargo run --locked --features ws-client --example hello_auth      # authenticate, then exit
cargo run --locked --features ws-client --example managed_quote   # recover state, quote RFQs
```

`ACTA_WS_URL` defaults to `wss://devnet-api.acta.markets`. `managed_quote` and
`quote` submit real signed quotes at a fixed demonstration price of
`1_000_000_000` (one quote token per underlying token), so use a test maker on
devnet. They do not price, hedge or fund.

### 4. The quote loop

This is the core of `examples/managed_quote.rs`: connect, authenticate, subscribe
to RFQs, sign a quote for each `RfqBroadcast`, and watch for fills and rejections.

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{maker::MakerQuoteClient, managed::*, types::*};
use std::{sync::Arc, time::Duration};
# async fn run(keypair: [u8; 64], owner: String) -> Result<(), Box<dyn std::error::Error>> {
static NONCES: AtomicNonceGenerator = AtomicNonceGenerator::new();

let signer = Arc::new(BytesSigner::from_keypair(&keypair)?); // quote-signing key
let maker_owner = decode_base58_32(&owner)?;

let config = ManagedWsConfig::new(
    "wss://devnet-api.acta.markets",
    HelloData {
        protocol_version: WS_PROTOCOL_VERSION.to_string(),
        features: vec![],
        client_name: Some("my-bot".to_string()),
        client_version: None,
    },
    signer.clone(),
)
.with_auth_pubkey(owner)
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
            let valid_until = QuoteExpiry::after(Duration::from_secs(350)).ok_or("clock")?;
            if valid_until.to_system_time().ok_or("expiry range")? > rfq.market.expiry_ts {
                continue;
            }
            let quote = RfqBinding::from_broadcast(rfq)?
                .quote()
                .maker_owner(maker_owner)
                .price(Price::new(1_000_000_000))
                .valid_until(valid_until)
                .nonce(Nonce::new(NONCES.next_u64()?))
                .sign(signer.as_ref())?;
            client.raw().send_in_epoch(ClientMessage::Quote(quote), epoch).await?;
        }
        ServerMessage::QuoteAcknowledged(ack) => println!("accepted {:?}", ack.order_id),
        ServerMessage::QuoteOutbid(outbid) => println!("outbid on {}", outbid.rfq_id),
        ServerMessage::QuoteRejected(rejected) => println!("rejected {:?}", rejected.reason),
        ServerMessage::QuoteFilled(fill) => println!("filled, position {}", fill.position_pda),
        ServerMessage::RfqClosed(closed) => println!("closed {:?}", closed.reason),
        _ => {}
    }
}
# Ok(())
# }
```

The sketch quotes only in its first connection epoch; after a reconnect
`send_in_epoch` returns `NotReady`. `managed_quote` shows the full pattern: wait
for the new epoch's recovery reads, then quote in it (see
[Readiness and reconnect](#readiness-and-reconnect)). A socket write is not an
acceptance; never replay old signed quotes after a reconnect.

### 5. After a fill

`QuoteFilled` means the position is open on chain. Optionally fund the
settlement leg with `FundPosition` ([Chain helpers](#chain-helpers)); a position
that expires in the money without it is closed by a permissionless liquidator
instead of settling with you. Settlement is run by Acta's keeper and pays the
owner's ATA directly; no SDK call is needed.

## Choose your path

- **Maker, managed session** (`ws-client`): `MakerQuoteClient` on `/maker` for
  quotes, cancellations and subscriptions, `MakerDataClient` on `/maker/data` for
  private reads; both reconnect and re-read state. Read `managed_quote`, then
  [Maker sessions](#maker-sessions) and [Quotes and signing](#quotes-and-signing).
- **Maker, own event loop** (`ws-client`): `WsClient` is a plain socket; you own
  auth, reconnect and recovery. Read `hello_auth`, `quote`, `discovery`.
- **Vault operator** (`vault`): `VaultOperatorClient` on `/vault` quotes for the
  vault, verifies and signs served opens and swaps, streams projection cursors
  and `VaultCatalogChanged`; `VaultManager` builds the capital-cycle crank.
  Read `vault_operator`, `vault_capital` and [Vault operators](#vault-operators).
- **Chain helpers** (`chain`, `chain-rpc`): premium deposit/withdrawal and
  position funding, account decoders. See [Chain helpers](#chain-helpers).
- **HTTP** (`http`): `HttpClient` for venue limits, tokens, markets and vaults.
  See [HTTP reference data](#http-reference-data).

## Features and examples

| Feature | Enables |
|---|---|
| `ws-client` | `MakerQuoteClient`, `MakerDataClient`, `ManagedWsHandle`, raw `WsClient` (Tokio) |
| `vault` | `VaultOperatorClient`, checked vault transaction signing, projection feed; includes `ws-client` and `chain` |
| `chain` | Instruction builders (maker premium and funding, `VaultManager`), derived addresses, account decoders, crank decision, Pyth price accounts, served-transaction verification; no WebSocket stack |
| `chain-rpc` | `ChainClient` over Solana RPC; extends `chain` |
| `http` | Read-only `HttpClient` (reqwest, rustls with native roots) |
| `test-helpers` | `ManagedWsHandle::test_handle`, message injection |

| Example | Feature | What it does |
|---|---|---|
| `hello_auth` | `ws-client` | Authenticates, prints success, closes |
| `managed_quote` | `ws-client` | Recovers maker state, submits epoch-bound quotes |
| `quote` | `ws-client` | Raw `/maker` handshake, RFQ subscription, signed quote |
| `discovery` | `ws-client` | Live quotes, positions, eligible markets and summary on `/maker/data` |
| `indicative` | `ws-client` | Answers unsigned indicative-price requests |
| `loopback_latency` | `ws-client` | Managed client against a local mock server; no keys or chain |
| `vault_operator` | `vault` | `/vault` session: quotes, verified fill and swap signing, projection cursor |
| `vault_capital` | `chain-rpc` | Capital-cycle crank: deposit window, equity, deposits, withdrawals, finalize |

Maker examples read `ACTA_KEYPAIR_PATH`, `ACTA_MAKER_OWNER` and `ACTA_WS_URL`.
`vault_operator` signs with the operator key in `ACTA_KEYPAIR_PATH` for
`ACTA_VAULT_PDA`. `vault_capital` also reads `ACTA_RPC_URL`, signs with the
vault's governance or delegate key and sends one capital step per pass.

```sh
ACTA_VAULT_PDA=<vault> cargo run --locked --features vault --example vault_operator
ACTA_VAULT_PDA=<vault> ACTA_RPC_URL=<rpc> cargo run --locked --features chain-rpc --example vault_capital
```

## Maker sessions

### Endpoints

`/maker` (the default) carries quotes, batches, replacements, cancellations and
subscriptions. RFQs need a server-side `Rfqs` subscription: `with_initial_subscribe`
makes it part of readiness; `subscribe_messages()` only attaches a local receiver.

A second connection on `/maker/data` serves private reads: `GetActiveRfqs`,
`GetMyQuotes`, `GetOrderStatus`, `GetMakerPositions`, `GetMarketsForMaker`,
`GetMyTrades`, `GetMmSummary`. `request()` returns a validated typed payload.

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{managed::*, maker::MakerDataClient, types::*};
use std::sync::Arc;
# async fn run(hello: HelloData) -> Result<(), Box<dyn std::error::Error>> {
# let signer = Arc::new(BytesSigner::from_secret([1u8; 32]));
let config = ManagedWsConfig::new("wss://devnet-api.acta.markets", hello, signer)
    .with_endpoint(MakerWsEndpoint::Data);
let data = MakerDataClient::spawn(config)?;
data.wait_until_ready().await?;
let summary = data
    .request(GetMmSummaryMessage { request_id: uuid::Uuid::new_v4() }, std::time::Duration::from_secs(2))
    .await?;
# let _ = summary;
# Ok(())
# }
```

`GetMyQuotes.scope` is required. `Live` is the complete current set, unpaged.
`History` is persisted records paged by `limit` and `cursor`/`cursor_id`
(`created_at`/`order_id`); positions and trades have their own pagination keys.

### Authentication

Managed clients validate the canonical Acta challenge before calling any signer,
also on reconnect. They authenticate as the signer's own key; when the
quote-signing key is not the owner, set `with_auth_pubkey(owner)` or the session
fails with `maker_not_registered`. Raw `WsClient` callers must call
`ws::auth::validate_maker_auth_challenge` before signing.

### Readiness and reconnect

Both endpoints go `Connected → Authenticated → Reconciled → Ready`. Every
authentication re-reads `MmSummary`, `ActiveRfqs` and `GetMyQuotes { scope: Live }`;
quote connections first restore subscriptions. The first local subscriber gets a
bounded backlog since spawn, so subscribe early.

`Ready` means the SDK received the reads, not that your strategy applied them,
and they are not one atomic snapshot. Apply recovery and lifecycle messages
before trading. Live quotes are `Active { rank }`, non-selectable `Retained`,
`AwaitingSignature` and `Executing`; historical records must not overwrite them.

Reconnect first tries `ResumeAuth`; only `session_expired` falls back to a full
challenge. Session replacement, version mismatch, rejected credentials and an
exhausted retry limit terminate the client (`Closed` / `Terminated`);
`invalid_signature` terminates after three consecutive rejections.
`max_reconnect_attempts = 0` retries forever.

Subscriptions, including removals and empty mint scopes, survive resume and fresh
auth; recovery re-sends both mint lists with fresh request IDs. Only one
subscription change may await an ACK at a time (`SubscriptionPending`).

Each inbound message carries an epoch and a per-connection sequence. A lagging
receiver gets `Gap` on its next `recv()`. `EndpointDefault` reconnects quote
sessions and surfaces gaps on data sessions; `Surface` never reconnects;
`Reconnect` stops sending and recovers.

### Sending and cancellation

Typed clients require `Ready`; so do raw trading commands. Every command is bound
to the epoch it was submitted in and is never replayed on a later connection.
`send()` confirms a socket write, not a server ACK; `try_send()` returns a
`SendTicket` (dropping it before dequeue cancels the send); `send_await()` waits
for the correlated response. Quotes, batches, replacements and cancellations
share one FIFO lane, which the backend preserves; control frames go first. A full
lane returns `QueueFull`.

Cancel with `raw().send_await(ClientMessage::CancelQuote(request), timeout)`:
`CancelQuoteAck` lists the order IDs actually cancelled, `CancelAllQuotesAck` the
aggregate. `QuoteCancelled` is a lifecycle event, not a receipt. A disconnect
between write and ACK leaves the outcome unknown. `CancelRfq` has no receipt:
`send()` it and wait for `RfqClosed`.

Keep each submitted `OrderId` until a cancellation, lifecycle fact or positive
lookup resolves it. `GetOrderStatus` returns `Pending`, `Confirmed { position_pda }`
or `Unknown`; neither `Unknown`, a timeout nor an empty `Live` result proves there
is no exposure.

### Cancel-on-disconnect

Quote connections require `cancel_on_disconnect` in `Welcome` by default and
terminate with `FeatureUnsupported` otherwise; opt out with
`with_cancel_on_disconnect(false)`. On disconnect the server removes Active and
retained quotes (the same set as `CancelAll`); selected and in-flight orders keep
their lifecycle. A gap-triggered reconnect also triggers it. COD has no bounded
cancellation time.

### Low-latency profile

`low_latency()` sets 2 s ping/pong deadlines, 1 s write timeout, a 1024 command
buffer, a 4096 inbound ring and gap policy `Reconnect`. Dead-socket detection
drops from about 40 s to about 4 s. Raw `WsClient` sets TCP_NODELAY; reuse
serialized bytes with `PreparedClientMessage` and `send_prepared()`.

## Quotes and signing

`RfqBinding::from_broadcast` checks the market PDA against the mints, expiry and
option type under `ACTA_PROGRAM_ID` before anything is signed; for another
deployment use `from_broadcast_for_program` with a locally configured program ID,
never one from the RFQ. `QuoteBuilder` derives the signed preimage and the
`QuoteMessage` from the same inputs. Your strategy still owns its mint allowlist,
pricing and decimals.

`GetActiveRfqs` is discovery data, not a signing binding: it carries only the
market address. Quote a validated broadcast, or verify the market and derive its
PDA yourself before using `compute_order_id` and the low-level signing functions.

- Reuse one `RfqBinding` per RFQ. `sign_with_async_signer` takes HSM or remote
  keys; `into_replacement(old_order_id)` makes a replacement without re-signing.
- The default strike is the requested one; `offered_strikes()` adds
  `order_options`, and `.strike(...)` rejects anything else.
- `Price` and `Strike` are 1e9 fixed-point per underlying unit, independent of
  mint decimals (`PRICE_SCALE`): `Price::new(2_000_000_000)` is 2 quote tokens per
  underlying. `Quantity` is underlying atomic units. Wrong scaling is not caught
  locally.
- `QuoteExpiry` is whole Unix seconds.
- With a quote-signing key that is not the owner, set `.maker_owner(owner)`;
  otherwise the server rejects the quote with `order_id preimage mismatch`.

A quote signs the 32-byte SHA-256 order ID of a fixed 182-byte preimage.
`preimage_args(maker_pubkey)` returns the exact `OrderPreimageArgs` the server
reconstructs:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::RfqBroadcastMessage;
# fn run(rfq: &RfqBroadcastMessage, signer: &BytesSigner) -> Result<(), Box<dyn std::error::Error>> {
let binding = RfqBinding::from_broadcast(rfq)?;
let builder = binding
    .quote()
    .price(Price::new(2_000_000_000))
    .valid_until(QuoteExpiry::from_unix_seconds(1_725_000_000))
    .nonce(Nonce::new(42));
let order_id = compute_order_id(&builder.preimage_args(signer.pubkey_bytes())?);
# let _ = order_id;
# Ok(()) }
```

`BytesSigner` zeroizes its key on drop. `SignerLike` is for local keys,
`AsyncSignerLike` for external processes or devices; every returned signature is
verified. `ManagedWsConfig::auth_timeout` covers signing, so signer futures must
be safe to cancel.

Server limits: 32 KiB per message, 50 quotes per batch, 50 quotes/s (burst 100)
per wallet, 30 messages/s (burst 60) per connection. Critical pushes (RFQs, ACKs,
fills) retry twice for 250 ms, then the server closes the connection to force
recovery.

## Vault operators

`/vault` carries the vault's permitted maker and taker trading, swaps,
subscriptions and projections on one connection. `examples/vault_operator.rs`
shows cursor handling and verified fill and swap signing.

- **Authentication:** `StartVaultAuth { operator, vault_pda }` → signed challenge
  bound to operator and vault → `VaultAuthSuccess { operator_kind, permissions }`.
  Every reconnect uses a fresh challenge; there is no `ResumeAuth`.
- **Grant:** `auth()` holds the current connection's grant and emits `None` on
  disconnect or `vault_session_invalidated`. `can_maker()` / `can_taker()` need
  `Ready` plus `SIGN_MAKER_QUOTES` / `OPEN_TAKER`. Governance quotes only while
  the same key holds an active primary-delegate grant with `SIGN_MAKER_QUOTES`.
- **Readiness:** each auth reads `GetMyActiveRfqs` and `GetPositions`; with maker
  permission also `GetMmSummary`, `GetActiveRfqs`, `GetMyQuotes { scope: Live }`.
- **Subscriptions:** only `WsChannel::Vaults`, no mint filters, on by default.
  RFQs arrive regardless. `VaultCatalogChanged { vault_pda }` means a vault was
  created, re-delegated or closed: re-list vaults.
- **Quoting:** `sign_quote` puts the vault in `maker_owner`. A writer vault with
  `OPEN_TAKER` can take a holder vault's quote; the order binds both vaults, the
  writer operator signs and pays, the holder delegate signs the quote.
- **Checked signing:** `verify_and_sign_sponsored_tx` and `verify_and_sign_swap`
  rebuild the expected instructions from what the operator agreed to
  (`VaultOpenIntent::{Sale, BetweenVaults, Fill}` with `VaultTradeTerms`, or
  `VaultSwapIntent` with request, quote, price updates, `router` and the
  operator's own oracle-based `min_output_floor`) and sign only if the served
  transaction matches: fee payer and signers, compute budget within
  `ComputeBudgetBounds`, every instruction's program, accounts and data, ATA
  creates, the maker's Ed25519 quote signature, and router accounts confined to
  the swap's route tail. A served minimum output below `min_output_floor` is
  rejected even within slippage. `router` is the one the program build pins:
  `JUPITER_V6_PROGRAM_ID`, or `DEVNET_SWAP_ROUTER_ID` on devnet. Deadlines are
  checked, the blockhash is not. v0 lookup tables come from
  `ChainClient::address_lookup_tables`. `verify_vault_open` / `verify_vault_swap`
  do the same for callers outside the operator client.
- **Operations:** rebalance needs `REBALANCE`; recovery swaps are governance-only.
  With `FUND_MAKER`, `VaultManager::fund_maker_position` moves the settlement leg
  (call: quote, put: underlying) from the vault into a position it holds.
  `VaultManager` also builds the capital-cycle crank (`examples/vault_capital.rs`).

A swap submit ACK confirms durable enqueue, not execution; after a lost ACK,
retry the same signed payload while the template is valid.
`VaultProjectionAdvanced` is a versioned cursor, not an execution result: ignore
duplicate or older versions and reload the HTTP vault detail on every newer
version or gap. `VaultProjection::follows(previous)` checks adjacency only.

## Chain helpers

With `chain-rpc`, `ChainClient::new(rpc_url, program_id, commitment)` sends the
owner's maker transactions (`deposit_premium`, `withdraw_premium`,
`fund_position`) or returns them via the `build_*_ixs` methods. `chain` alone has
the offline builders. `FundPositionArgs` can wrap native SOL for wSOL positions:

```rust,no_run
# #[cfg(feature = "chain-rpc")]
# {
use acta_maker_sdk::chain::{FundPositionArgs, NativeSolFunding};
# fn args(maker_owner: solana_sdk::pubkey::Pubkey, position_pda: solana_sdk::pubkey::Pubkey) {
let fund = FundPositionArgs {
    maker_owner,
    position_pda,
    create_atas: true,
    native_sol: NativeSolFunding::with_default_reserve(),
};
# let _ = fund;
# }
# }
```

The pre-sign budget covers wrap amount, rent and fee (only wrap and reserve with
another fee payer). It does not lock SOL against concurrent spending, and RPC
errors are never read as a zero balance. Use `NativeSolFunding::Disabled` to
wrap externally.

## HTTP reference data

`HttpClient` reads `venue()` (program ID, minimum quote validity, settlement
buffer, refresh lead, RFQ and signature timeouts, per-RFQ and per-taker limits),
`tokens()` (by mint; underlyings add size rule, strike tick and quote mints),
`markets(&MarketFilter)` (by underlying, quote, expiry, side and `active`;
`Some(false)` adds closed and finalized markets) and `vaults()` / `vault(pda)` /
`vault_positions(pda)`. Read limits from here instead of hardcoding them.
Responses are revalidated with `If-None-Match`, so polling costs a 304; refetch
when `subscribe_reference_changes()` yields new ETags.

## Wire notes

Known string values and tagged states (`PositionType`, `MakerQuoteState`,
`OrderExecutionState`, `QuoteCancelReason`, `RfqCloseReason`, ...) are SDK
enums; see the API reference. Free-form reason text stays a string.
`Welcome.server_time_unix_ms`, `AuthSuccess.expires_at`,
`RfqBroadcast.sent_at_unix_ms` and `instruction_index` on chain events are
required. `*_unix_ms` is milliseconds; credential expiry is Unix seconds and does
not bound the socket. Chain events are keyed by `(signature, instruction_index)`.
`GlobalStats.usd` amounts are exact decimal strings; legacy numeric volume fields
are raw units, not USD.

## Latency budget

`examples/loopback_latency.rs`, release build, `low_latency()`, 20 000 samples per
leg, Apple silicon. This bounds SDK overhead; network and server time add on top.

| Leg | p50 | p90 | p99 |
|-----|-----|-----|-----|
| `try_send` → frame on the peer's socket | 20 µs | 27 µs | 60 µs |
| peer send → parsed by the session task (paced 1/ms) | 150 µs | 281 µs | 424 µs |
| session task → subscriber `recv()` | 19 µs | 39 µs | 70 µs |

## Upgrading and development

Breaking changes per release are in `CHANGELOG.md`, which ships with the crate.
In a repository checkout, `bash scripts/test.sh` runs the wire-compatibility
check (needs `../rust-backend` or `ACTA_BACKEND_DIR`), feature combinations,
tests, examples, Clippy, rustdoc and packaging.

## Documentation

- [API reference](https://docs.rs/acta-maker-sdk)
- [Acta integration documentation](https://beta.acta.markets/docs)

## License

MIT
