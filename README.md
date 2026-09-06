# acta-maker-sdk

Rust SDK for quoting Acta options: managed WebSocket connections, RFQ-bound
order signing, typed protocol messages, and Solana funding instructions.

| API | Use it for |
| --- | --- |
| `MakerQuoteClient` | Quotes, replacements and cancellations on `/maker` |
| `MakerDataClient` | Private reads and execution lookups on `/maker/data` |
| `ManagedWsHandle` | Correlated requests and explicit connection epochs |
| `WsClient` | Raw transport with application-managed authentication and recovery |

## Install

Requires Rust 1.89 or later. Default features are empty.

```toml
[dependencies]
acta-maker-sdk = { version = "0.4.0", features = ["ws-client"] }
```

| Feature | What it enables |
|---|---|
| `ws-client` | Typed maker clients, `ManagedWsHandle`, and raw `WsClient`; requires Tokio |
| `chain` | Solana instruction builders (`DepositPremium`, `WithdrawPremium`, `FundPosition`) |
| `chain-rpc` | Solana RPC queries (extends `chain`) |
| `test-helpers` | Test utilities (`ManagedWsHandle::test_handle`, message injection) |

## Onboarding

Acta's admin registers the maker owner and any delegated auth or quote-signing
keys on-chain through governance. Request onboarding through the
[integration documentation](https://docs.acta.markets/quickstart/maker-rust-sdk)
before connecting. Registration is permissioned; `maker_not_registered` ends
the connection attempt without retries.

## Connect and subscribe

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

RFQ broadcasts require the `Rfqs` server subscription. `with_initial_subscribe`
includes it in SDK readiness; `subscribe_messages()` only attaches a local
receiver. Apply recovery messages to your strategy before starting to quote,
even after `wait_until_ready()` returns.

### Separate quote and data connections

`ManagedWsConfig` defaults to the maker quote endpoint (`/maker`). Keep
latency-sensitive commands there: `Quote`, `BatchQuotes`, `ReplaceQuote`,
`CancelQuote`, `CancelAllQuotes`, and subscriptions. For private maker reads
and recovery queries, open a separate data-plane connection:

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

Use the data endpoint (`/maker/data`) for
`GetActiveRfqs`, `GetMyQuotes`, `GetOrderStatus`, `GetMakerPositions`,
`GetMarketsForMaker`, `GetMyTrades`, and `GetMmSummary`. `GetMyQuotes.scope`
is required: `Live` returns the complete current owner set without pagination;
`History` pages persisted records with `limit` and the `cursor`/`cursor_id`
pair (`created_at` plus `order_id`). Positions and trades remain paginated by
their existing keys. A Live response always has `has_more: false`.
`MakerDataClient::request()` moves the typed payload out of the received
message, so reading it is infallible; `into_payload()` takes it by value.

## Readiness and reconnect

Both endpoints advance `Connected → Authenticated → Reconciled → Ready` after
receiving `MmSummary`, `ActiveRfqs` and `GetMyQuotes { scope: Live }`.
Quote connections restore subscriptions first. The first subscriber can read a
bounded since-spawn backlog; subscribe early to avoid overflowing it.

Use `connect_timeout` and `max_reconnect_attempts` to bound connection failures;
`0` retries forever. `reconnect_jitter_ratio` spreads reconnect attempts.

After an authenticated disconnect, the managed client first resumes with the
server-issued session ID. Only `session_expired` falls back to a full challenge
authentication on the same socket; other resume errors remain failures for that
attempt. `session_replaced`, protocol-version mismatch, rejected credentials, and
an exhausted reconnect limit are terminal states, visible through
`ManagedWsState::Closed` and `ManagedWsEvent::Terminated`. The one exception is
`invalid_signature`: it retries with the normal backoff and only goes terminal
on the third rejection with no successful authentication in between, so a
one-off signer or server glitch does not stop an unattended maker.

Dropping the last `ManagedWsHandle` also stops the connection task. Call
`close().await` when you need to wait until shutdown has completed.

Typed clients reject commands before readiness with `NotReady`.
Use `client.raw()` for protocol operations not exposed by the typed API.

## Delivery and cancellations

Once a session is `Ready`, the session task reads and correlates responses while
one writer owns the socket sink. Quotes, batches, replacements and cancellations
share a FIFO lane. Ping, authentication and close use the control lane. The
backend also applies quote and cancellation requests in their connection order.
Concurrent producers are ordered by actor receipt.
A full writer lane reports `QueueFull`. `write_timeout` limits each socket write;
it does not bound the time a cancellation spends behind earlier quotes.

`send()` confirms a socket write, not a server ACK. `send_await()` waits for a
correlated response. `try_send()` returns a `SendTicket`; `wait()` reports its
socket-write result.

To await cancellation on the server, use `client.raw().send_await(ClientMessage::CancelQuote(request), timeout)`
and match `CancelQuoteAck` or `RequestError`. The ACK contains the request ID,
RFQ ID and actual cancelled order IDs. `CancelAllQuotesAck` contains the actual
aggregate. `QuoteCancelled` is a lifecycle event and never completes an awaited
cancellation. Losing the connection after a write but before an ACK leaves the
outcome unknown; the SDK does not replay trading commands. An empty
`QuoteCancelled.order_ids` removes nothing. Lifecycle RFQ/order versions and
cancellation order IDs are required fields; missing values are rejected during
decoding rather than replaced with zero or an empty set.

`CancelRfq` has no correlated success receipt. Use `send()` and process
`RfqClosed` as lifecycle state; it can also result from expiry or a fill.
`send_await(CancelRfq)` returns `NoCorrelationKey` without writing the command.
A rejected cancellation arrives as `RequestError` with its `request_id`.

### Connection epochs and application recovery

Trading commands, including those sent through the raw managed handle, require
Ready. All commands submitted from Ready, including reads, are bound to that
connection epoch and cannot be sent by a later connection. Typed data requests
check readiness and capture that epoch together before enqueueing. The low-level `WsClient` remains
application-managed.

Receive and apply recovery responses and lifecycle messages before restarting
your strategy. `Ready` confirms that the SDK received the required responses;
it does not confirm that the strategy applied them, and the reads are not an
atomic snapshot. `GetMyQuotes { scope: Live }` contains every Core-held
unfinished quote for the owner: selectable `Active { rank }`, non-selectable
`Retained` (including locked non-winners and frozen refresh quotes),
`AwaitingSignature`, and `Executing`. `History` is a separate source and must
not overwrite this live state.

Keep submitted `OrderId` intent until an exact cancellation/replacement ID, a
confirmed lifecycle fact, or a positive execution lookup resolves it.
`GetOrderStatus` is available through `MakerDataClient::request()` and the
ready quote handle's raw correlated request path; it returns `Pending`,
`Confirmed { position_pda }`, or
`Unknown`. `Unknown`, a timeout, and an empty Live result do not prove that a
maker is flat.
When application recovery is tracked separately from SDK readiness, use
`ManagedWsHandle::send_in_epoch(message, connection_epoch)` with the epoch of
the recovery snapshot the application applied. A stale or non-ready epoch is
rejected with `NotReady` before enqueueing. The queued command keeps that epoch
and cannot be sent by a later connection; disconnect/write errors still apply.

### Slow consumers

Each inbound message carries a connection epoch and a connection-local sequence.
A lagging receiver reports `Gap`. `GapPolicy::EndpointDefault` reconnects quote
sessions and only surfaces gaps on data sessions. With reconnect enabled, once
the session actor processes the signal, it stops
sending and reconnects through recovery. A frame already handed to the writer
may have reached the server. Explicit `GapPolicy::Surface` reports the gap
without reconnecting. Lag is detected when the receiver next calls `recv()`.

### Cancel-on-disconnect (COD)

Managed quote connections request `cancel_on_disconnect` by default. The server
must include it in `Welcome.enabled_features`, otherwise the SDK terminates with
`FeatureUnsupported`. To disable COD, set `with_cancel_on_disconnect(false)`
and omit `cancel_on_disconnect` from `HelloData.features`. The setter does not
remove manually supplied features.
Data connections do not request cancellation for the quote session.

COD removes the session's Active quotes and retained non-selectable quotes after
the server detects the connection loss and Core processes it.
CancelAll removes the same set, optionally filtered by market, and its ACK lists
the removed IDs. Those quotes cannot return through rollback. Selected/in-flight
orders remain governed by their lifecycle, including when their maker cancels
other strikes. Single-RFQ CancelQuote rejects a locked RFQ as a whole.
With `cancel_on_disconnect` enabled, the same cleanup applies when a `Gap`
causes a reconnect.

For an application-requested stop, `close()` uses the control lane; an ongoing
socket write can still delay it. A slowly draining quote queue does not by itself
force a disconnect. COD therefore supplies a disconnect fallback, not a bounded
cancellation time.

## Subscriptions

Dynamic subscriptions survive resume and fresh authentication, including removals and empty mint filters.
Only one subscription change may await acknowledgement at a time; another
returns `SubscriptionPending`. Trading and cancellation remain available.
The acknowledgement timeout starts when the subscription frame is written.

`initial_subscribe` seeds the desired target; its `request_id` is not sent on
the wire. Recovery generates fresh request IDs. A new handle starts with empty
desired channels/scopes when no initial target is supplied. It does not adopt
server-persisted mint filters: recovery sends both mint lists explicitly and
clears a persisted scope when its desired list is empty. Supply the intended
filters even when resuming an existing server session.

## Latency and limits

### Client profile

`ManagedWsConfig::low_latency()` sets ping every 2 s with a 2 s pong deadline, a 1 s write
timeout, 1024 queued commands, a 4096-slot inbound ring
and `GapPolicy::Reconnect`. The defaults stay conservative for tooling that
polls occasionally. The heartbeat settings detect a dead socket in about 4 s
with `low_latency()` and up to 40 s with defaults, subject to task scheduling.

### Server limits

Default limits are 32 KiB per client message, 50 quotes per batch, 50 quotes/s
with a burst of 100 per wallet, and 30 messages/s with a burst of 60 per
connection. Critical pushes (`RfqBroadcast`, ACKs, fills) retry for 250 ms twice;
if delivery still fails, the server closes the connection for recovery.

### SDK loopback measurements

Measured by `examples/loopback_latency.rs` on one machine, release build,
`low_latency()` profile, 20 000 samples per leg (Apple silicon, Tokio
multi-thread). These are local measurements, not a cancellation-latency guarantee;
network and server time come on top.

| Leg | p50 | p90 | p99 |
|-----|-----|-----|-----|
| `try_send` → frame on the peer's socket (one frame in flight) | 20 µs | 27 µs | 60 µs |
| peer send → message parsed by the session task (paced 1/ms) | 150 µs | 281 µs | 424 µs |
| session task → subscriber `recv()` | 19 µs | 39 µs | 70 µs |

The outbound leg is the command channel, lane routing, serialization and the
socket write. The inbound leg includes the peer's own wake-up after pacing, so
it overstates the SDK's share. Under a burst the same legs turn into queueing
time, which is what `QueueFull` and `Gap` are for.

## Raw WebSocket access

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

A quote signs the 32-byte order ID: SHA-256 of a fixed 182-byte preimage.
`RfqBinding` builds the preimage and `QuoteMessage` from the same strike, price,
expiry and nonce, avoiding `order_id preimage mismatch` errors.

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::RfqBroadcastMessage;
use std::time::Duration;
# fn run(rfq: &RfqBroadcastMessage) -> Result<(), Box<dyn std::error::Error>> {
# let signer = BytesSigner::from_secret([1u8; 32]);
# let nonce = 42u64;
let quote = RfqBinding::from_broadcast(rfq)?
    .quote()
    .price(Price::new(2_000_000_000))
    .valid_until(QuoteExpiry::after(Duration::from_secs(350)).ok_or("clock before epoch")?)
    .nonce(Nonce::new(nonce))
    .sign(&signer)?;
# let _ = quote;
# Ok(()) }
```

`RfqBinding::from_broadcast` decodes the RFQ's program, market and taker keys
once; reuse one binding for every quote and replacement on that RFQ.
`sign_with_async_signer` covers keys held in an HSM or a remote service.
Either way, `quote.into_replacement(old_order_id)` turns the signed quote into a
`ReplaceQuoteMessage`.

By default the builder quotes the RFQ's requested strike. `offered_strikes()`
lists every strike the RFQ will accept — its requested one plus its
`order_options` — and `.strike(...)` selects among them. A strike outside that
set is refused locally instead of after a round trip.

`valid_until` is a `QuoteExpiry` of whole Unix seconds, because that is what the
preimage signs and what the wire carries. Converting from a `SystemTime`
truncates; a sub-second fraction cannot survive to be rounded into a different
second than the one that was signed.

`Price` and `Strike` are 1e9 fixed-point per one unit of the underlying,
independent of mint decimals (`PRICE_SCALE` is that factor). `Quantity` is in
the underlying mint's atomic units. `Price::new(2_000_000_000)` quotes a gross
premium of 2 quote tokens per underlying unit; the SDK cannot detect a price
supplied in the wrong units.

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

A maker registration can delegate signing: an auth key that opens sessions and a
quote key that signs orders, both distinct from the maker owner. The pubkey you
authenticate as is always the *owner* — the server looks the registration up by
it and verifies the challenge against the registered auth signing key — and the
order preimage is always reconstructed with the owner's key, so both sides must
be told the owner when the keys differ:

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

The SDK validates the position, market, and existing wSOL account. When the maker
is also the fee payer, the pre-sign check includes the wrap amount, rent for token
accounts created by funding, the assembled transaction's RPC fee, and the configured
reserve. With an external fee payer, the maker budget includes only the wrap and
reserve. This is a snapshot, not an on-chain lock: another wallet transaction can
still spend SOL after the check. RPC and account-validation errors fail closed;
they are not treated as a zero token balance. Use
`NativeSolFunding::Disabled` when wrapping is handled by the caller.

## Settlement

Settlement is a permissionless on-chain crank, normally run by Acta's keeper
once the market is finalized. Proceeds are paid directly into the maker owner's
associated token account for the payout mint — the keeper's transaction creates
that account if it is missing — so a settled position requires no maker action
and no SDK call. `build_withdraw_premium_ixs` moves earned premium out of the
premium vault; it is not part of settlement.

## Protocol types

The current WS contract requires `Welcome.server_time_unix_ms`,
`AuthSuccess.expires_at`, `RfqBroadcast.sent_at_unix_ms`, and `instruction_index`
on all six known chain events. Missing or null values are rejected. The two
`*_unix_ms` fields remain milliseconds; credential expiry remains Unix seconds.
Credential expiry does not itself define the lifetime of an authenticated socket.
Chain events are identified by `(signature, instruction_index)`, never by
signature alone.

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

The programs in the
[`examples/` directory](https://github.com/acta-markets/rust-maker-sdk/tree/main/examples)
cover raw authentication, discovery, quoting, indicative pricing, and the
managed quote client.

```bash
cargo run --example managed_quote --features ws-client
```

## Documentation

See the [Rust API reference](https://docs.rs/acta-maker-sdk/0.4.0) and the
[maker integration guide](https://docs.acta.markets/quickstart/maker-rust-sdk).
README examples are compiled as `no_run` doctests without opening live connections.

## License

MIT
