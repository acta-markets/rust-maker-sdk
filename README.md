# acta-maker-sdk

Rust SDK for Acta options market makers. Handles WebSocket connectivity, authentication, order signing, and message serialization.

Two API levels:

- **`ManagedWs`** — reconnects and authenticates automatically, and exposes `send_await()` for correlated responses.
- **`WsClient`** — raw WebSocket client. You manage the connection yourself. Good for scripts and testing.

No features enabled by default — lightweight core with opt-in WS and Solana support.

> WebSocket snippets are compiled with `cargo test --doc --features ws-client`; chain snippets are
> also compiled with `cargo test --doc --features ws-client,chain-rpc`. They are marked `no_run`:
> they type-check but do not open a live connection.

## Install

```toml
[dependencies]
acta-maker-sdk = { version = "0.2.0", features = ["ws-client"] }
```

| Feature | What it enables |
|---|---|
| `ws-client` | WebSocket client (`WsClient`, `ManagedWs`), requires tokio |
| `chain` | Solana instruction builders (`DepositPremium`, `WithdrawPremium`, `FundPosition`) |
| `chain-rpc` | Solana RPC queries (extends `chain`) |
| `test-helpers` | Test utilities (`ManagedWsHandle::test_handle`, message injection) |

## Quick start (ManagedWs)

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{managed::*, types::*};
use std::sync::Arc;
# fn run() -> Result<(), Box<dyn std::error::Error>> {
let signer = BytesSigner::from_secret([1u8; 32]);
let signer_for_auth = signer.clone();

let config = ManagedWsConfig::new(
    "wss://devnet-api.acta.markets/maker",
    HelloData {
        protocol_version: WS_PROTOCOL_VERSION.to_string(),
        features: vec![],
        client_name: Some("my-bot".to_string()),
        client_version: Some("0.1.0".to_string()),
    },
    signer.pubkey_base58(),
    Arc::new(move |challenge: &str| {
        Ok(signer_for_auth.sign_message_base58(challenge.as_bytes()))
    }),
);

let handle = spawn_managed_ws(config)?;
// handle.send(), handle.subscribe_messages(), handle.send_await()
# let _ = handle;
# Ok(())
# }
```

`ManagedWsConfig` defaults to the maker quote endpoint (`/maker`). Keep
latency-sensitive commands there: `Quote`, `BatchQuotes`, `ReplaceQuote`,
`CancelQuote`, `CancelAllQuotes`, and subscriptions. For private maker reads
and recovery queries, open a separate data-plane connection:

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{managed::*, types::*};
use std::sync::Arc;
# fn run() {
# let signer = BytesSigner::from_secret([1u8; 32]);
# let signer_for_auth = signer.clone();
# let hello = HelloData {
#     protocol_version: WS_PROTOCOL_VERSION.to_string(),
#     features: vec![],
#     client_name: None,
#     client_version: None,
# };
# let challenge_signer = Arc::new(move |challenge: &str| {
#     Ok(signer_for_auth.sign_message_base58(challenge.as_bytes()))
# });
let data_config = ManagedWsConfig::new(
    "wss://devnet-api.acta.markets",
    hello,
    signer.pubkey_base58(),
    challenge_signer,
)
.with_endpoint(MakerWsEndpoint::Data);
# let _ = data_config;
# }
```

The data endpoint normalizes to `/maker/data` and is the preferred place for
`GetActiveRfqs`, `GetMyQuotes`, `GetMakerPositions`, `GetMarketsForMaker`,
`GetMyTrades`, and `GetMmSummary`. Automatic reconcile reads are enabled on
the data endpoint and disabled on the quote endpoint by default.

`send()` confirms that the frame was written to the current WebSocket. It is
not a server ACK. Use `send_await()` when the command has a correlated server
response. Commands submitted while reconnecting fail with `Disconnected`; the
SDK never replays a quote behind the strategy's back. `try_send()` returns a
`SendTicket`, whose `wait()` method reports the eventual socket-write result.

Each inbound message carries a connection epoch and a connection-local
sequence number. `ManagedMessageReceiver::recv()` returns `Gap` if its bounded
broadcast buffer overflows. Treat that as a reason to refresh state through the
data connection.

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

## Order signing

```rust,no_run
use acta_maker_sdk::*;
# fn run() {
# let signer = BytesSigner::from_secret([1u8; 32]);
// Field values come from the `RfqBroadcast` you are quoting.
let args = OrderPreimageArgs {
    chain_id: 0,
    program_id: [0u8; 32],
    is_taker_buy: false,
    position_type: PositionType::CoveredCall,
    market: [0u8; 32],
    strike: 100_000_000_000,
    quantity: 1,
    gross_price: 2_000_000_000,
    valid_until: 1_725_000_000,
    maker: signer.pubkey_bytes(),
    taker: [0u8; 32],
    nonce: 42,
};
let order_id = compute_order_id(&args);
let signature = sign_order_id_with_signer(&order_id, &signer);
# let _ = signature;
# }
```

`BytesSigner` wraps an Ed25519 keypair with `Zeroize` on drop. `SignerLike` is the synchronous
hot-path interface for local keys. Use `AsyncSignerLike` when signing may wait for an external
process or device. The SDK verifies every returned signature against the requested message and
`pubkey_bytes`. `ManagedWsConfig::auth_timeout` bounds the complete authentication flow, including
signing; timed-out signer futures are dropped and must therefore be cancellation-safe.

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
is also the fee payer, the final pre-sign check includes the wrap amount, rent for
missing token accounts created by funding, the RPC fee for the assembled transaction, and the configured
reserve. With an external fee payer, the maker budget includes only the wrap and
reserve. This is a snapshot guarantee, not an on-chain lock: another concurrent
wallet transaction can still spend SOL after the check. RPC and account-validation
errors fail closed; they are not treated as a zero token balance. Use
`NativeSolFunding::Disabled` when wrapping is handled by the caller.

## Value sets (string enums)

These are `String` on the wire; the SDK provides enums for known value sets:

- `PositionType`: `"covered_call"` | `"cash_secured_put"`
- `QuoteStatus`: `"pending"` | `"best"` | `"outbid"` | `"filled"` | `"expired"`
- `QuoteCancelReason`: `"requested"` | `"risk_check"` | `"rfq_accepted"`
- `RfqAvailableAgainReason`: `"signature_timeout"` | `"tx_failed"` | `"tx_build_failed"`
- `RfqCloseReason`: `"expired"` | `"taker_cancelled"` | `"filled"` | `"market_expired"` | `"ladder_timeout"`
- `QuoteFinalStatus`: `"expired"` | `"outbid"` | `"cancelled"` | `"filled"`
- `PositionUpdateType`: `"created"` | `"funded"` | `"liquidated"` | `"settled"`

Known order and position statuses are represented by SDK enums. Truly open-ended backend fields,
such as diagnostic reason text, remain strings.

## Examples

Examples require the `ws-client` feature:

```bash
cargo run --example hello_auth --features ws-client
```

### Subscribe to RFQs + chain events

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::*;
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;
client
    .subscribe(SubscribeData {
        request_id: Default::default(),
        channels: vec![WsChannel::Rfqs, WsChannel::ChainEvents],
        underlying_mints: None,
        quote_mints: None,
    })
    .await?;
# Ok(()) }
```

### Send a quote

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::types::*;
use uuid::Uuid;
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
# let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;
client
    .quote(QuoteMessage {
        rfq_id: Uuid::new_v4(),
        strike: Strike::new(100_000_000_000),
        price: Price::new(2_000_000_000),
        valid_until: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_725_000_000),
        nonce: Nonce::new(42),
        order_id: OrderId::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap(),
        signature: "base58-signature".to_string(),
    })
    .await?;
# Ok(()) }
```

### Discovery APIs

```rust,no_run
use acta_maker_sdk::*;
use acta_maker_sdk::ws::{managed::normalize_maker_data_ws_url, types::*};
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let url = normalize_maker_data_ws_url("wss://devnet-api.acta.markets");
let mut client = WsClient::connect(&url).await?;

client
    .get_my_quotes(GetMyQuotesMessage {
        request_id: Default::default(),
        active_only: true,
        limit: None,
    })
    .await?;

client
    .get_maker_positions(GetMakerPositionsMessage {
        request_id: Default::default(),
        market: None,
        underlying_mint: None,
        status: Some(vec!["open".to_string(), "funded".to_string()]),
        min_expiry_ts: None,
        limit: None,
    })
    .await?;
# Ok(()) }
```

### Cancel quote(s)

```rust,no_run
use acta_maker_sdk::*;
use uuid::Uuid;
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
# let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;
client
    .cancel_quote(Uuid::new_v4())
    .await?;
client
    .cancel_all_quotes(Some("market_pda_base58".to_string()))
    .await?;
# Ok(()) }
```

## Documentation

Full documentation is available at [docs.acta.markets](https://docs.acta.markets):

- Rust SDK integration guide
- Wire examples (JSON and Rust)
- Maker API reference (all WS messages)
- Sandbox / devnet setup
- FAQ and troubleshooting

## License

MIT
