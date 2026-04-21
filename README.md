# acta-maker-sdk

Rust SDK for Acta options market makers. Handles WebSocket connectivity, authentication, order signing, and message serialization.

Two API levels:

- **`ManagedWs`** — auto-reconnect, auto-auth, `send_await()` for request-response. Use this for production bots.
- **`WsClient`** — raw WebSocket client. You manage the connection yourself. Good for scripts and testing.

No features enabled by default — lightweight core with opt-in WS and Solana support.

## Install

```toml
[dependencies]
acta-maker-sdk = { version = "0.1.0", features = ["ws-client"] }
```

| Feature | What it enables |
|---|---|
| `ws-client` | WebSocket client (`WsClient`, `ManagedWs`), requires tokio |
| `chain` | Solana instruction builders (`DepositPremium`, `WithdrawPremium`, `FundPosition`) |
| `chain-rpc` | Solana RPC queries (extends `chain`) |
| `test-helpers` | Test utilities (`ManagedWsHandle::test_handle`, message injection) |

## Quick start (ManagedWs)

```rust
use acta_maker_sdk::*;
use acta_maker_sdk::ws::managed::*;
use acta_maker_sdk::ws::types::*;
use std::sync::Arc;

let signer = BytesSigner::from_keypair(&keypair_bytes);
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

let handle = spawn_managed_ws(config);
// handle.send(), handle.subscribe_messages(), handle.send_await()
```

## Quick start (WsClient)

```rust
use acta_maker_sdk::ws::{client::WsClient, types::*};
use acta_maker_sdk::WS_PROTOCOL_VERSION;

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
                challenge: data.challenge,
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
```

## Order signing

```rust
let args = OrderPreimageArgs { /* ... from RfqBroadcast ... */ };
let order_id = compute_order_id(&args);
let signature = sign_order_id_with_signer(&order_id, &signer);
```

`BytesSigner` wraps an Ed25519 keypair with `Zeroize` on drop. Implement `SignerLike` to plug in HSM/KMS.

## Value sets (string enums)

These are `String` on the wire; the SDK provides enums for known value sets:

- `PositionType`: `"covered_call"` | `"cash_secured_put"`
- `QuoteStatus`: `"pending"` | `"best"` | `"outbid"` | `"filled"` | `"expired"`
- `QuoteCancelReason`: `"requested"` | `"risk_check"` | `"rfq_accepted"`
- `RfqAvailableAgainReason`: `"signature_timeout"` | `"tx_failed"` | `"tx_build_failed"`
- `RfqCloseReason`: `"expired"` | `"taker_cancelled"` | `"filled"` | `"market_expired"` | `"ladder_timeout"` | `"order_failed"` | `"signature_timeout"`
- `QuoteFinalStatus`: `"expired"` | `"outbid"` | `"cancelled"` | `"filled"`
- `PositionUpdateType`: `"created"` | `"funded"` | `"liquidated"` | `"settled"`

Some status fields are raw strings from the backend (e.g. `OrderStatusMessage.status`, `PositionInfo.status`).

## Examples

Examples require the `ws-client` feature:

```bash
cargo run --example hello_auth --features ws-client
```

### Subscribe to RFQs + chain events

```rust
use acta_maker_sdk::ws::{client::WsClient, types::*};

let mut client = WsClient::connect("wss://devnet-api.acta.markets/maker").await?;
client
    .subscribe(SubscribeData {
        channels: vec![WsChannel::Rfqs, WsChannel::ChainEvents],
        underlying_mints: None,
        quote_mints: None,
    })
    .await?;
```

### Send a quote

```rust
use acta_maker_sdk::ws::{client::WsClient, types::*};

client
    .quote(QuoteMessage {
        rfq_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        strike: Strike::new(100_000_000_000),
        price: Price::new(2_000_000_000),
        valid_until: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_725_000_000),
        nonce: 42,
        order_id: OrderId::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap(),
        signature: "base58-signature".to_string(),
    })
    .await?;
```

### Discovery APIs

```rust
use acta_maker_sdk::ws::{client::WsClient, types::*};

client
    .get_my_quotes(GetMyQuotesMessage {
        active_only: true,
    })
    .await?;

client
    .get_maker_positions(GetMakerPositionsMessage {
        market: None,
        underlying_mint: None,
        status: Some(vec!["open".to_string(), "funded".to_string()]),
        min_expiry_ts: None,
    })
    .await?;
```

### Cancel quote(s)

```rust
use acta_maker_sdk::ws::{client::WsClient, types::*};

client
    .cancel_quote(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
    .await?;
client
    .cancel_all_quotes(CancelAllQuotesMessage {
        market: Some("market_pda_base58".to_string()),
    })
    .await?;
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
