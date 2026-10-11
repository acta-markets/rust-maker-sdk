# acta-maker-sdk

Rust SDK for Acta makers and vault operators: authentication, WebSocket sessions,
order signing and Solana instructions. Features are opt-in; defaults are empty.

| API | Purpose |
|---|---|
| `MakerQuoteClient`, `MakerDataClient` | Managed maker quote and data connections |
| `VaultOperatorClient` | Managed vault connection |
| `ManagedWsHandle` | Protocol access, reconnect and correlated requests |
| `WsClient` | Raw WebSocket; application owns the lifecycle |

## Install

Requires Rust 1.89 or later. For maker applications:

```toml
[dependencies]
acta-maker-sdk = { version = "0.4.3-vaults.1", features = ["ws-client"] }
```

For the vault API:

```toml
[dependencies]
acta-maker-sdk = { version = "0.4.3-vaults.1", features = ["vault"] }
```

Build quotes with `RfqBinding::from_broadcast`: it verifies the market PDA from
the mints, expiry and option type under `ACTA_PROGRAM_ID` before signing. For
another deployment use `from_broadcast_for_program` with a locally configured
program ID, never one copied from the RFQ. Your strategy still chooses its mint
allowlist and trusted pricing/decimal inputs. Low-level `compute_order_id` and
signing functions operate on caller-verified inputs.

`GetActiveRfqs` is discovery data, not a validated signing binding: it contains
only the market address. Quote a validated broadcast, or independently verify
the market descriptor and derive its PDA before using low-level signing APIs.

| Feature | What it enables |
|---|---|
| `ws-client` | Typed maker clients, `ManagedWsHandle`, and raw `WsClient`; requires Tokio |
| `vault` | Vault operator client, transaction signing and projection feed (includes `ws-client`) |
| `chain` | Solana instruction builders (`DepositPremium`, `WithdrawPremium`, `FundPosition`) |
| `chain-rpc` | Solana RPC queries (extends `chain`) |
| `vault-chain` | Vault capital-cycle instructions (`VaultManager`) and derived vault addresses, account decoders, crank decision and Pyth price-account derivation (`vault` + `chain`) |
| `http` | Read-only `HttpClient` for the public vault, market, token and venue endpoints (reqwest, rustls with native roots). `venue()` returns server limits (minimum quote validity, RFQ timeout bounds, settlement buffer); `tokens()` returns per-underlying size rules, strike tick and Pyth feed ids. Read these instead of hardcoding them. |
| `test-helpers` | Test utilities (`ManagedWsHandle::test_handle`, message injection) |

Managed clients validate the canonical Acta authentication challenge before invoking
any signer, including during reconnect. Maker authentication supports a registered
signing key different from the owner (`with_auth_pubkey`); vault authentication
binds the challenge to both the configured operator and vault. Raw `WsClient`
callers must use `ws::auth::validate_maker_auth_challenge` or
`ws::auth::validate_vault_auth_challenge` before signing. Server challenge bytes
and order signatures are unchanged; existing backend versions remain compatible.

## Onboarding

Acta governance registers the maker owner and optional delegated auth/quote keys
on chain. Request access through the [integration documentation](https://beta.acta.markets/docs).
There is no self-service registration; an unregistered key receives terminal
`maker_not_registered`.

## Quick start (typed maker clients)

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{managed::*, maker::MakerQuoteClient, types::*};
use std::sync::Arc;
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let signer = Arc::new(BytesSigner::from_secret([1u8; 32]));

let config = ManagedWsConfig::new(
    "wss://devnet-api.acta.markets/maker",
    HelloData {
        protocol_version: WS_PROTOCOL_VERSION.to_string(),
        features: vec![],
        client_name: Some("my-bot".to_string()),
        client_version: Some("0.1.0".to_string()),
    },
    signer,
)
.with_cancel_on_disconnect(true)
.low_latency()
.with_initial_subscribe(SubscribeData {
    request_id: uuid::Uuid::new_v4(),
    channels: vec![WsChannel::Rfqs],
    underlying_mints: None,
    quote_mints: None,
});

let client = MakerQuoteClient::spawn(config)?;
let mut messages = client.subscribe_messages();
let connection_epoch = client.wait_until_ready().await?;
# let _ = (messages, connection_epoch);
# Ok(())
# }
```

RFQs require a server-side `Rfqs` subscription. `with_initial_subscribe` includes
it in readiness; `subscribe_messages()` only attaches a local receiver.

`/maker` is the default quote endpoint: quotes, batches, replacements,
cancellations and subscriptions. Use a separate `/maker/data` connection for
private reads:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{managed::*, maker::MakerDataClient, types::*};
use std::sync::Arc;
# use uuid::Uuid;
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
# let signer = Arc::new(BytesSigner::from_secret([1u8; 32]));
# let hello = HelloData {
#     protocol_version: WS_PROTOCOL_VERSION.to_string(),
#     features: vec![],
#     client_name: None,
#     client_version: None,
# };
let data_config = ManagedWsConfig::new("wss://devnet-api.acta.markets", hello, signer)
    .with_endpoint(MakerWsEndpoint::Data);
let data = MakerDataClient::spawn(data_config)?;
data.wait_until_ready().await?;
let summary = data
    .request(
        GetMmSummaryMessage {
            request_id: Uuid::new_v4(),
        },
        std::time::Duration::from_secs(2),
    )
    .await?;
# let _ = summary;
# Ok(())
# }
```

`/maker/data` supports `GetActiveRfqs`, `GetMyQuotes`, `GetOrderStatus`,
`GetMakerPositions`, `GetMarketsForMaker`, `GetMyTrades` and `GetMmSummary`.
`MakerDataClient::request()` returns a validated typed payload; `into_payload()`
takes ownership.

`GetMyQuotes.scope` is required:

- `Live`: complete current owner set, unpaged, `has_more: false`.
- `History`: persisted records, paged by `limit` and `cursor`/`cursor_id`
  (`created_at`/`order_id`). Positions and trades use their own pagination keys.

### Readiness and reconnect

Both maker endpoints follow `Connected → Authenticated → Reconciled → Ready`.
Every authentication re-reads `MmSummary`, `ActiveRfqs` and
`GetMyQuotes { scope: Live }`; quote connections first restore subscriptions.
The first local subscriber receives a bounded since-spawn backlog. Subscribe
early to retain recovery responses.

`Ready` means the SDK received the reads, not that the strategy applied them;
they are not an atomic snapshot. Apply recovery and lifecycle messages before
resuming trading. Live quotes include `Active { rank }`, non-selectable
`Retained` (locked non-winners and frozen refresh quotes), `AwaitingSignature`
and `Executing`. Historical records must not overwrite live state.

An authenticated maker reconnect first attempts `ResumeAuth` with its session ID.
Only `session_expired` falls back to a full challenge on the same socket.
Other errors fail the attempt. Session replacement, version mismatch, rejected
credentials and an exhausted retry limit terminate the client, reported by
`Closed` / `Terminated`. `invalid_signature` retries with backoff and terminates
after three consecutive rejections without successful authentication.
`connect_timeout` bounds connection attempts; `max_reconnect_attempts = 0`
retries forever. `reconnect_jitter_ratio` spreads attempts.

Dynamic subscriptions survive resume and fresh auth, including removals and
empty mint scopes. `initial_subscribe` seeds the desired target; recovery uses
fresh request IDs, never its initial ID. Without an initial target, desired
channels/scopes are empty. Recovery sends both mint lists explicitly, clearing
persisted filters when the desired lists are empty. Supply the intended filters
even when resuming. `auto_reconcile` and its setter are no longer available.

Only one subscription mutation may await an ACK; another returns
`SubscriptionPending`. Trading/cancellation remain available. Its acknowledgement
timeout starts after the frame is written.

Each inbound message has an epoch and connection-local sequence. A lagging
receiver reports `Gap` on its next `recv()`. `EndpointDefault` reconnects quote
sessions and surfaces gaps on data sessions. `Surface` never reconnects; with
`Reconnect`, the session stops sending and starts recovery when it processes the
gap. Frames already passed to the writer may have reached the server.

### Sending and cancellation

Typed clients require `Ready`; `raw()` exposes additional managed operations.
Raw managed trading commands also require readiness. Every command submitted
from `Ready`, including reads, is bound to that connection epoch. Typed data
requests check readiness and capture the epoch together. Commands cannot move
to a later connection, and trading commands are never replayed.

Use `send_in_epoch(message, epoch)` when the strategy tracks the recovery epoch
it has applied. A stale or non-ready epoch returns `NotReady` before enqueueing;
disconnect and write errors still apply. Raw `WsClient` is application-managed.

`send()` confirms a socket write, not a server ACK. `try_send()` returns a
`SendTicket`; keep it until `wait()` completes, since dropping it before dequeue
cancels the send. `send_await()` waits for a correlated response.

One task reads/correlates; one writer owns the socket. Quotes, batches,
replacements and cancellations share FIFO order; concurrent producers are
ordered by actor receipt. Control frames (ping, auth, close) take priority.
The backend also preserves connection order for quotes and cancellations.
A full lane returns `QueueFull`; `write_timeout` bounds each write, not time
spent behind queued quotes.

For cancellation, call `raw().send_await(ClientMessage::CancelQuote(request), timeout)`
and match `CancelQuoteAck` or `RequestError`. The ACK contains request ID, RFQ ID
and actual cancelled order IDs; `CancelAllQuotesAck` contains the actual aggregate.
`QuoteCancelled` is a lifecycle event, not an awaited receipt; an empty
`order_ids` removes nothing. Missing lifecycle RFQ/order versions or cancellation
IDs fail decoding. A disconnect after write but before ACK leaves the outcome
unknown.

`CancelRfq` has no correlated success receipt. Use `send()` and handle `RfqClosed`,
which may also mean expiry or fill. `send_await(CancelRfq)` returns
`NoCorrelationKey` without writing; rejection arrives as `RequestError`.

Retain each submitted `OrderId` until an exact cancellation/replacement ID,
confirmed lifecycle fact or positive execution lookup resolves it.
`GetOrderStatus` works through typed data requests and the ready quote handle's
raw correlated path. It returns `Pending`, `Confirmed { position_pda }` or
`Unknown`. Neither `Unknown`, a timeout nor an empty Live result proves no exposure.

Dropping the last handle stops the connection task; `close().await` waits for
shutdown. Queued data-lane frames are discarded; an ongoing write can delay close.

### Cancel-on-disconnect

Managed quote connections require `cancel_on_disconnect` in `Welcome` by default;
otherwise they terminate with `FeatureUnsupported`. Opt out with
`with_cancel_on_disconnect(false)` and omit `cancel_on_disconnect` from
`HelloData.features`; the setter does not remove manually supplied features.
Data connections do not request it.

After the server detects disconnect and Core processes it, COD removes Active
and retained non-selectable quotes. `CancelAll` removes the same set, optionally
by market, and reports the removed IDs; rollback cannot restore them.
Selected/in-flight orders retain their lifecycle, including when other strikes
are cancelled. `CancelQuote` rejects a locked RFQ as a whole. A gap-triggered
reconnect also triggers negotiated COD.

Close uses the control lane, but may wait for an ongoing write. A slow queue
alone does not disconnect: COD does not guarantee a bounded cancellation time.

### Low-latency profile

`low_latency()` sets ping/pong deadlines to 2 s, write timeout to 1 s, command
buffer to 1024, inbound ring to 4096 and gap policy to `Reconnect`.
Approximate dead-socket detection is 4 s, versus 40 s with defaults.

## Quick start (WsClient)

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::*;
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
# let signer = BytesSigner::from_secret([1u8; 32]);
let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;

client.send_hello(HelloData {
    protocol_version: WS_PROTOCOL_VERSION.to_string(),
    features: vec!["quote_expired".to_string()],
    client_name: Some("maker-bot".to_string()),
    client_version: Some("0.1.0".to_string()),
}).await?;

while let Some(msg) = client.next().await {
    match msg? {
        ServerMessage::AuthRequest(data) => {
            acta_maker_sdk::ws::auth::validate_maker_auth_challenge(&data.challenge)?;
            client.auth_challenge(AuthChallengeData {
                challenge: data.challenge.clone(),
                signature: signer.sign_message_base58(data.challenge.as_bytes()),
                pubkey: signer.pubkey_base58(),
            }).await?;
        }
        ServerMessage::AuthSuccess(data) => {
            println!("authenticated: {}", data.session_id);
        }
        other => println!("server: {other:?}"),
    }
}
# Ok(()) }
```

`WsClient` enables TCP_NODELAY and applies bounded frame, message, and write
buffers. For a message that will be sent more than once, build a
`PreparedClientMessage` and call `send_prepared()` to reuse its serialized
bytes.

## Quoting an RFQ

`RfqBinding` and `QuoteBuilder` derive the signed preimage and `QuoteMessage`
from the same strike, price, expiry and nonce, preventing `order_id preimage mismatch`:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::RfqBroadcastMessage;
use std::time::Duration;
# fn run(rfq: &RfqBroadcastMessage) -> Result<(), Box<dyn std::error::Error>> {
# let signer = BytesSigner::from_secret([1u8; 32]);
# let nonce = 42u64;
let valid_until = QuoteExpiry::after(Duration::from_secs(350)).ok_or("clock before epoch")?;
if valid_until.to_system_time().ok_or("quote expiry exceeds SystemTime")? > rfq.market.expiry_ts {
    return Ok(());
}
let quote = RfqBinding::from_broadcast(rfq)?
    .quote()
    .price(Price::new(2_000_000_000))
    .valid_until(valid_until)
    .nonce(Nonce::new(nonce))
    .sign(&signer)?;
# let _ = quote;
# Ok(()) }
```

Reuse one `RfqBinding` per RFQ to decode program, market and taker keys once.
`sign_with_async_signer` supports HSM/remote keys; `into_replacement(old_order_id)`
adds the superseded order ID without signing again.

The default strike is the RFQ's requested one. `offered_strikes()` adds
`order_options`; `.strike(...)` rejects anything outside that set.
`QuoteExpiry` uses whole Unix seconds; `SystemTime` conversion truncates so
preimage and wire expiry match.

`Price` and `Strike` use 1e9 fixed-point per underlying unit, independent of mint
decimals (`PRICE_SCALE`). `Quantity` uses underlying atomic units.
`Price::new(2_000_000_000)` means 2 quote tokens per underlying unit.
Incorrect scaling is not rejected by local validation.

### Signing the preimage yourself

`preimage_args(maker_pubkey)` returns the exact `OrderPreimageArgs` the server
will reconstruct, for inspection or for signing out of band:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::RfqBroadcastMessage;
# fn run(rfq: &RfqBroadcastMessage) -> Result<(), Box<dyn std::error::Error>> {
# let signer = BytesSigner::from_secret([1u8; 32]);
let binding = RfqBinding::from_broadcast(rfq)?;
let builder = binding
    .quote()
    .price(Price::new(2_000_000_000))
    .valid_until(QuoteExpiry::from_unix_seconds(1_725_000_000))
    .nonce(Nonce::new(42));

let args = builder.preimage_args(signer.pubkey_bytes())?;
let order_id = compute_order_id(&args);
# let _ = order_id;
# Ok(()) }
```

`BytesSigner` wraps an Ed25519 keypair with `Zeroize` on drop. `SignerLike` is the
synchronous interface for local keys; use `AsyncSignerLike` when signing may wait
for an external process or device. The SDK verifies every returned signature
against the requested message and `pubkey_bytes`. `ManagedWsConfig::auth_timeout`
also covers signing, so timed-out signer futures must be cancellation-safe.

### Delegated signing keys

Auth and quote signing keys may differ from the registered maker owner.
Authentication identifies the owner but uses the registered auth key's signature;
the quote preimage also identifies the owner. Set both owner overrides:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::RfqBroadcastMessage;
# fn run(rfq: &RfqBroadcastMessage, owner: [u8; 32]) -> Result<(), Box<dyn std::error::Error>> {
# let quote_signer = BytesSigner::from_secret([1u8; 32]);
let quote = RfqBinding::from_broadcast(rfq)?
    .quote()
    .price(Price::new(2_000_000_000))
    .valid_until(QuoteExpiry::from_unix_seconds(1_725_000_000))
    .nonce(Nonce::new(42))
    .maker_owner(owner)
    .sign(&quote_signer)?;
# let _ = quote;
# Ok(()) }
```

On the connection side, both `ManagedWsConfig::new` and `new_async` default
to the signer's own key; override it with `with_auth_pubkey(owner)`. Without
`maker_owner` every quote from a delegated key is rejected as
`order_id preimage mismatch`; without the owner auth pubkey the session fails as
`maker_not_registered`.

## Vault operator session

Enable `vault` for `/vault`: permitted maker/taker trading, swaps,
subscriptions and projections share one connection.

- **Subscriptions:** only `WsChannel::Vaults`, without mint filters; other
  requests return `invalid_vault_subscription`. RFQs arrive through the vault
  registry independently of channel subscriptions. `Vaults` is subscribed by
  default; unsubscribe to opt out.
- **Authentication:** `StartVaultAuth { operator, vault_pda }` → signed challenge
  → `VaultAuthSuccess { operator_kind, permissions }`. Every reconnect uses a
  fresh challenge; vault sessions have no `ResumeAuth`.
- **Grant:** `auth()` holds the current connection's grant, possibly before reads
  finish. It clears on disconnect/invalidation; the watch emits `None`.
  `vault_session_invalidated` starts fresh auth even if the old socket stays open.
  `can_maker()` / `can_taker()` require `Ready` and the granted
  `SIGN_MAKER_QUOTES` / `OPEN_TAKER` bit. Governance can quote when the same key
  has an active primary-delegate grant with `SIGN_MAKER_QUOTES`; revocation or
  expiry removes quoting without removing governance authority. This requires
  the matching backend update. The server and contract check each action.
- **Signing:** the operator signs challenges and prepared transactions.
  Maker quotes require an active primary-delegate grant; `sign_quote` puts the vault address
  in `maker_owner`. With `vault-chain`, `verify_and_sign_sponsored_tx` and
  `verify_and_sign_swap` rebuild the expected instructions from what the operator
  agreed to (`VaultOpenIntent::Sale` / `BetweenVaults` / `Fill` with
  `VaultTradeTerms`; `VaultSwapIntent` with the request, quote, price-update
  accounts and `min_output_floor`, the operator's own oracle-based minimum output;
  a served `minimum_out` below it is rejected even when it fits the quote's slippage) and sign only if the served transaction matches: fee payer and
  signers, compute budget within `ComputeBudgetBounds`, every instruction's
  program, accounts (signer/writable) and data, canonical idempotent ATA
  creates, the maker's Ed25519 quote signature, and a Jupiter route confined to
  the swap instruction's tail (at most 64 accounts, no signer, no payer, no
  read-only vault account made writable). The blockhash is not checked. v0
  lookup tables are resolved from the tables passed in; `ChainClient::address_lookup_tables`
  fetches active tables for `served_lookup_table_keys`. Errors name the first mismatch.
  `verify_vault_open` / `verify_vault_swap` run the same checks without signing,
  e.g. for an external taker filling a vault quote. Deadlines (`signature_deadline`,
  `valid_until`) are checked locally. `_unverified` helpers skip all of this and
  trust the server.
- **Readiness:** each auth fetches `GetMyActiveRfqs` and `GetPositions`.
  A grant with maker permission also fetches `GetMmSummary`, `GetActiveRfqs`
  and `GetMyQuotes { scope: Live }`.
- **Two vaults:** a writer with governance/`OPEN_TAKER` accepts a holder vault's
  `SIGN_MAKER_QUOTES` quote through the same RFQ messages. The order binds both
  vault addresses. The writer operator signs and pays for the transaction;
  the holder delegate signs the quote. No hot co-signature is added. The vault
  signing helper locates the transaction signature slot by signer public key.
- **Operations:** rebalance requires `REBALANCE`; the manager constructs and submits the transaction without a hot co-signature; recovery
  swaps are governance-only.

```rust,no_run
# #[cfg(feature = "vault-chain")]
# {
use acta_maker_sdk::*;
use acta_maker_sdk::chain::vault::{ComputeBudgetBounds, VaultManager, VaultSwapIntent};
use acta_maker_sdk::ws::types::*;
use std::sync::Arc;
# fn load_vault_manager() -> Result<VaultManager, Box<dyn std::error::Error>> { unimplemented!() }
# fn vault_price_updates() -> Result<(solana_sdk::pubkey::Pubkey, solana_sdk::pubkey::Pubkey), Box<dyn std::error::Error>> { unimplemented!() }
# fn oracle_min_output() -> Result<u64, Box<dyn std::error::Error>> { unimplemented!() }
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let operator = Arc::new(BytesSigner::from_secret([1u8; 32]));
let config = VaultOperatorConfig::new(
    "wss://devnet-api.acta.markets/vault",
    HelloData {
        protocol_version: WS_PROTOCOL_VERSION.to_string(),
        features: vec![],
        client_name: Some("vault-operator".to_string()),
        client_version: None,
    },
    "<vault pda>",
    operator,
)
.with_managed(|managed| managed.low_latency());

let client = VaultOperatorClient::spawn(config)?;
let mut projections = client.subscribe_projections();
client.wait_until_ready().await?;

// `VaultManager::from_vault` over the on-chain vault account; price updates from its oracle sources.
let vault: VaultManager = load_vault_manager()?;
let (main_price_update, second_price_update) = vault_price_updates()?;
// Lowest output the operator accepts, priced from its own oracle, not from the server's quote.
let min_output_floor: u64 = oracle_min_output()?;
let request = SwapQuoteData {
    request_id: uuid::Uuid::new_v4(),
    vault_pda: client.vault_pda().to_string(),
    mode: VaultSwapMode::Rebalance,
    direction: VaultSwapDirection::MainToSecond,
    amount: "1000".to_string(),
    slippage_bps: 50,
};
let quote = client.swap_quote(request.clone()).await?;
let lookup_tables: Vec<solana_sdk::message::AddressLookupTableAccount> = Vec::new();
let submitted = client.swap_submit(client.verify_and_sign_swap(
    &VaultSwapIntent {
        vault: &vault,
        request: &request,
        quote: &quote,
        main_price_update,
        second_price_update,
        min_output_floor,
    },
    &lookup_tables,
    ComputeBudgetBounds { max_unit_limit: 600_000, max_unit_price_micro_lamports: 50_000 },
)?).await?;
tracing::info!(execution = ?submitted.execution_id, "swap queued");

let step = projections.recv().await?;
tracing::info!(version = step.version(), "reload vault HTTP detail");
# Ok(())
# }
# }
```

Swap submit ACKs confirm durable enqueue, not chain execution.
After a lost ACK, retry the same signed payload and execution identity while
the prepared template remains valid.

`VaultProjectionAdvanced` is a versioned cursor, not an execution result.
Ignore duplicate/older versions; reload HTTP vault detail on every newer version
or stream gap. `VaultProjection::follows(previous)` checks adjacency only.
`examples/vault_operator.rs` shows cursor handling and sponsored signing
(with the `_unverified` helpers, so it is not production signing);
the application supplies the HTTP reload and risk accounting.

## Funding positions with native SOL

With the `chain-rpc` feature, `FundPositionArgs` can wrap native SOL when the
position settles in wSOL. Wrapping is opt-in:

```rust,no_run
# #[cfg(feature = "chain-rpc")]
# {
use acta_maker_sdk::chain::{FundPositionArgs, NativeSolFunding};
# use solana_sdk::pubkey::Pubkey;
# fn args(maker_owner: Pubkey, position_pda: Pubkey) {
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

The SDK validates the position, market and existing wSOL account. If the maker
pays fees, the pre-sign budget includes wrap amount, new token-account rent,
RPC transaction fee and reserve. With another fee payer it includes only wrap
and reserve. This snapshot does not lock SOL against concurrent spending.
RPC/account errors are not treated as zero balance. Use
`NativeSolFunding::Disabled` when wrapping externally.

## Settlement

After market finalization, Acta's keeper normally runs permissionless settlement.
Proceeds go directly to the maker owner's payout ATA, created by the keeper
transaction if needed. No maker SDK call is required. `build_withdraw_premium_ixs`
withdraws premium separately.

## Wire value sets

The SDK provides enums for known string values and tagged state objects:

- `PositionType`: `"covered_call"` | `"cash_secured_put"`
- `MakerQuoteState`: `active { rank, rfq_version }`, `retained { rfq_version }`, `awaiting_signature { rfq_version }`, `executing { rfq_version }`, or `historical { status }`; `QuoteRank` is `"best"` | `"outbid"` and `HistoricalQuoteStatus` is `"submitted"` | `"filled"` | `"cancelled"` | `"replaced"` | `"expired"` | `"lost"`.
- `OrderExecutionState`: `pending`, `confirmed { position_pda }`, or `unknown`.
- `QuoteCancelReason`: `"requested"` | `"risk_check"` | `"rfq_accepted"` | `"maker_disconnected"`
- `RfqAvailableAgainReason`: `"signature_timeout"` | `"tx_failed"` | `"tx_build_failed"`
- `RfqCloseReason`: `"expired"` | `"taker_cancelled"` | `"filled"` | `"market_expired"` | `"ladder_timeout"`
- `QuoteFinalStatus`: `"expired"` | `"outbid"` | `"cancelled"` | `"filled"`
- `PositionUpdateType`: `"created"` | `"funded"` | `"liquidated"` | `"settled"`

Known order and position statuses use SDK enums. Open-ended fields, such as
diagnostic reason text, remain strings.

## Examples

From a checkout of this repository, point the examples at the JSON keypair for
your registered signing key (a standard 64-byte Solana JSON array). Registration
and funding are separate setup steps; these examples do not create or fund a maker.

```sh
export ACTA_KEYPAIR_PATH=/absolute/path/to/maker-keypair.json
cargo run --locked --features ws-client --example hello_auth
```

`ACTA_WS_URL` defaults to `wss://devnet-api.acta.markets`. If the registered maker
owner differs from the signing key, set `ACTA_MAKER_OWNER` to the owner's base58
public key; it is used for authentication and order preimages.

| Example | What it does |
|---|---|
| `hello_auth` | Completes authentication, prints success, closes the connection |
| `discovery` | Reads live quotes, open/funded positions, eligible markets and the maker summary on `/maker/data` |
| `managed_quote` | Recovers maker state and submits epoch-bound quotes through `MakerQuoteClient` |
| `quote` | Raw `/maker` handshake, RFQ subscription and signed quote submission |
| `indicative` | Answers unsigned indicative-price requests on `/maker` |
| `loopback_latency` | Managed client against a local mock server; no credentials or chain access |
| `vault_operator` | `/vault` session: quotes, sponsored-fill signing and the projection feed |
| `vault_capital` | Capital-cycle crank over RPC: deposit window, equity, deposits, withdrawals, finalize |

```sh
cargo run --locked --release --features ws-client --example loopback_latency
ACTA_VAULT_PDA=<vault> cargo run --locked --features vault --example vault_operator
ACTA_VAULT_PDA=<vault> ACTA_RPC_URL=<rpc> cargo run --locked --features vault-chain,chain-rpc --example vault_capital
```

`vault_operator` signs with the operator key from `ACTA_KEYPAIR_PATH`
for the vault in `ACTA_VAULT_PDA`. `vault_capital` signs with the vault's governance
or delegate key and sends one capital step per pass.

The quote examples submit real signed quotes at a fixed demonstration price of
`1_000_000_000` (one quote token per underlying token); the indicative example
uses `1_500_000_000`. Use a test maker on devnet. The examples do not implement
pricing, hedging or funding automation. A socket-write completion is not a quote
acceptance or an on-chain fill; after reconnect, wait for recovered state in the
new connection epoch and do not replay old signed quotes.

## Upgrading from 0.4.2

`QuoteExpiry::to_system_time()` returns `None` outside the platform's date range;
use `as_unix_seconds()` when no platform-date conversion is needed. The infallible
`From<QuoteExpiry> for SystemTime` conversion was removed. Referral counters use
`u64` instead of `i64`. Order-signing bytes and the WebSocket protocol version
are unchanged.

## Signing and server limits

Quotes sign the 32-byte SHA-256 order ID derived from a fixed 182-byte preimage.
`SignerLike` / `AsyncSignerLike` support delegated keys and HSMs.

Server limits: 32 KiB per client message, 50 quotes per batch, 50 quotes/s with
burst 100 per wallet, and 30 messages/s with burst 60 per connection. Critical
pushes (RFQs, ACKs, fills) retry for 250 ms twice, then close the connection to
force recovery.

## Latency budget

Measured by `examples/loopback_latency.rs` on one machine, release build,
`low_latency()` profile, 20 000 samples per leg (Apple silicon, Tokio
multi-thread). Loopback numbers bound the SDK's own overhead; network and
server time come on top.

| Leg | p50 | p90 | p99 |
|-----|-----|-----|-----|
| `try_send` → frame on the peer's socket (one frame in flight) | 20 µs | 27 µs | 60 µs |
| peer send → message parsed by the session task (paced 1/ms) | 150 µs | 281 µs | 424 µs |
| session task → subscriber `recv()` | 19 µs | 39 µs | 70 µs |

Outbound includes queueing, routing, serialization and socket write; inbound
includes the paced peer's wake-up. Bursts add queueing, surfaced by `QueueFull`
and `Gap`.

## USD trading statistics

`GlobalStats.usd`, when provided by the server, separates `notional_24h` from
`premium_24h`. Both are exact decimal dollar strings; use a decimal parser for
exact accounting. `priced_trades_24h` counts confirmed trades
with stored oracle valuations. Compare it with `total_trades_24h` for coverage.
The snapshot uses submission-time prices, including the quote token's USD price;
missing historical prices are not reconstructed. Legacy numeric volume/price
fields retain their raw-unit meanings and are not USD values.

## Development checks

```sh
bash scripts/test.sh
```

This checks backend wire compatibility, feature combinations, tests, examples,
Clippy, rustdoc and the Cargo package. The wire check needs the matching backend
checkout at `../rust-backend`, or `ACTA_BACKEND_DIR` set to its path. Package
verification runs offline after the build has populated the dependency cache.
It does not publish the crate or connect to a deployed Acta service.

With `../acta-contract` checked out, `cargo test --manifest-path scripts/vault-parity/Cargo.toml`
checks vault instruction builders against `acta-contract-sdk`.

## Documentation

See the [Rust API reference](https://docs.rs/acta-maker-sdk) and the
[Acta integration documentation](https://beta.acta.markets/docs).

## Wire metadata

The current WS contract requires `Welcome.server_time_unix_ms`,
`AuthSuccess.expires_at`, `RfqBroadcast.sent_at_unix_ms`, and `instruction_index`
on all six known chain events. Missing or null values are rejected. The two
`*_unix_ms` fields remain milliseconds; credential expiry remains Unix seconds.
Credential expiry does not itself define the lifetime of an authenticated socket.
Chain events are identified by `(signature, instruction_index)`, never by
signature alone.

## License

MIT
