# acta-maker-sdk

Rust SDK for Acta options market makers: WebSocket clients, quote signing and
Solana funding instructions.

## Install

Requires Rust 1.89 or later.

```toml
[dependencies]
acta-maker-sdk = { version = "0.4.3", features = ["ws-client"] }
```

Use `MakerQuoteClient` to submit, replace and cancel quotes, and `MakerDataClient`
to read maker state. They handle authentication, reconnects and subscriptions;
your application supplies pricing and manages risk. Maker registration is required.

Default features are empty. Enable `ws-client` for WebSocket clients,
`chain` for Solana instruction builders, or `chain-rpc` for RPC helpers.

The crate version is `0.4.3`; the WebSocket protocol version remains `1.0.0`
(`WS_PROTOCOL_VERSION`). They are separate version numbers.

## Run the examples

From a checkout of this repository, point the examples at the JSON keypair for
your registered maker signing key. Registration and funding are separate setup
steps; these examples do not create or fund a maker.

```sh
export ACTA_KEYPAIR_PATH=/absolute/path/to/maker-keypair.json
export ACTA_WS_URL=wss://devnet-api.acta.markets
cargo run --locked --features ws-client --example hello_auth
```

`ACTA_WS_URL` defaults to the devnet endpoint. If the registered maker owner
differs from the signing key, also set `ACTA_MAKER_OWNER` to the owner's base58
public key. The examples use that owner for authentication and order preimages,
and sign with the key from the file. The keypair file is a standard Solana JSON
array of 64 bytes.

| Example | What it does |
|---|---|
| `hello_auth` | Completes authentication, prints success, closes the connection |
| `discovery` | Reads live quotes, the first page of open/funded positions, eligible markets and the maker summary on `/maker/data` |
| `managed_quote` | Recovers maker state and submits epoch-bound quotes through `MakerQuoteClient`; handles reconnects |
| `quote` | Demonstrates the raw `/maker` handshake, RFQ subscription and signed quote submission |
| `indicative` | Answers unsigned indicative-price requests on `/maker` |
| `loopback_latency` | Measures the managed client against a local mock server; no credentials or chain access |

Replace `hello_auth` in the command to run another network example. For the
local benchmark:

```sh
cargo run --locked --release --features ws-client --example loopback_latency
```

The quote examples submit real signed quotes using a fixed demonstration price
of `1_000_000_000` (one quote token per underlying token). The indicative example
uses `1_500_000_000`. Use a test maker on devnet; replace these prices with your
strategy and select the markets you intend to quote before using real funds.
The examples do not implement a pricing model, hedging or funding automation.

`hello_auth`, `discovery`, `quote` and `indicative` show a single raw connection.
`discovery` prints response frames; use `MakerDataClient::request` for typed
responses correlated to individual requests. Use managed clients for automatic
reconnect and state recovery. A socket-write
completion is not a quote acceptance or an on-chain fill: handle the corresponding
server events. After reconnect, wait for recovered state in the new connection
epoch; do not replay old signed quotes blindly.

## Authentication and signing

Managed clients validate the canonical Acta maker-auth challenge before invoking
any signer, including during reconnect and repeated authentication. If you use
`WsClient` directly, call `ws::auth::validate_maker_auth_challenge` before signing
an `AuthRequest`; the examples demonstrate this. An arbitrary server string must
never reach the order signer. The authenticated maker owner may differ from its
registered signing key (`with_auth_pubkey`). Server challenge bytes and order
signatures are unchanged.

Build quotes with `RfqBinding::from_broadcast`: it verifies the market PDA from
the mints, expiry and option type under `ACTA_PROGRAM_ID` before signing. For
another deployment use `from_broadcast_for_program` with a locally configured
program ID, never one copied from the RFQ. Your strategy still chooses its mint
allowlist and trusted pricing/decimal inputs. Low-level `compute_order_id` and
signing functions operate on caller-verified inputs.

`GetActiveRfqs` is discovery data, not a validated signing binding: it contains
only the market address. Quote a validated broadcast, or independently verify
the market descriptor and derive its PDA before using low-level signing APIs.

## Upgrading from 0.4.2

`QuoteExpiry` stores whole Unix seconds as `u64`. `to_system_time()` returns
`None` outside the platform's date range; use `as_unix_seconds()` when no
platform-date conversion is needed.

The infallible `From<QuoteExpiry> for SystemTime` conversion was removed.
Referral counters now use `u64` instead of `i64`. See the changelog for the
remaining fixes; order-signing bytes and the WebSocket protocol version are
unchanged.

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

## Documentation

- [Integration documentation](https://beta.acta.markets/docs)
- [Examples](https://github.com/acta-markets/rust-maker-sdk/tree/main/examples)
- [Changelog](https://github.com/acta-markets/rust-maker-sdk/blob/main/CHANGELOG.md)

## License

MIT — see [LICENSE](https://github.com/acta-markets/rust-maker-sdk/blob/main/LICENSE).
