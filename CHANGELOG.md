# Changelog

## 0.4.3-vaults.1

- The `http` feature adds `http::HttpClient`, a read-only client for the public `/api/v1/vaults`, `/api/v1/vaults/{pda}` and `/api/v1/markets` routes with wire-exact `Vault` and `Market` types.
- `can_maker()` accepts governance with a current `SIGN_MAKER_QUOTES` grant. The backend requires an active primary-delegate grant even when governance and delegate share a key; revoked or expired grants cannot quote.

- `SwapQuoteData` carries `direction: VaultSwapDirection` (`main_to_second` / `second_to_main`) instead of `input_mint` / `output_mint`.
- `RfqRequest`, `AcceptQuote`, `SubmitSignedSponsoredTx` and `CancelRfq` now wait for a Ready session like other trading messages.
- The `vault-chain` feature adds `chain::vault`: `VaultManager` builds `SetAcceptingDeposits`, `ProcessVaultEquity` (and the idle-withdrawal variant), `ProcessDeposits`, `ProcessWithdrawals`, `ProcessFinalWithdrawal` and `FinalizeVaultCycle`; batches are capped at `VAULT_CAPITAL_BATCH_USERS` (three users fit the 1232-byte packet) and sized with `vault_capital_batch_compute_units` over `deposit_batch_shape_with_destinations` / `withdrawal_batch_shape_with_destinations`.
- Vault governance instructions: `propose_update`, `apply_update`, `cancel_update`, `rotate_delegate`, `disable_delegate`.
- `VaultAccount` / `VaultRequest` decoders, `oracle_source_pyth_feed` and `pyth_price_feed_account`.
- Crank decision `next_step`, `WindowTracker` and `plan_withdrawals`; `examples/vault_capital.rs` runs the cycle over RPC.
- Checked transaction signing decodes once, compares the locally approved message before deadlines, and preserves other signature slots for legacy/v0 transactions.
- `VaultSwapMode::Closing` encodes `closing` for governance-controlled terminal second-to-main conversion; normal maker/RFQ messages are unchanged.
- Projection adjacency rejects older, duplicate and wrapped versions; the operator example keeps its cursor monotonic.
- Concurrent vault swap failures retain their own correlation, including out-of-order `trading_paused`.
- Vault grants clear on connection end, failed recovery, or task panic. `VaultOperatorClient::can_maker()` / `can_taker()` require Ready and the current connection grant; reconnect replaces grants.
- The `vault` feature adds `VaultOperatorConfig` / `VaultOperatorClient` on `/vault`: `StartVaultAuth` signs with the operator key on each reconnect (never `ResumeAuth`); taker reads reconcile on every auth and maker reads when the delegate may quote; `sign_quote` uses the vault as `maker_owner`; `sign_sponsored_tx` / `sign_swap` check the expected message and reject expired deadlines locally (`_unverified` variants skip the message check); `swap_quote` and `swap_submit` correlate; `subscribe_projections()` streams `VaultProjectionAdvanced`; `MakerWsEndpoint::Vault` and `ManagedWsHandle::vault_auth()` expose the path.
- Vault-only wire types: `StartVaultAuth`, `SwapQuote`, `SwapSubmit` (`ClientMessage`); `VaultAuthSuccess`, `VaultProjectionAdvanced`, `SwapQuoteResult`, `SwapSubmitResult` (`ServerMessage`); `WsEndpointKind::Vault`, `WsChannel::Vaults`, `VaultOperatorKind`, `VaultDelegatePermissions`, `VaultSwapMode`, and `VaultSwapDirection`. Makers are unchanged.

## 0.4.3 - 2026-09-18

- Native SOL funding reads the maker and position ATAs in one RPC when both are needed. Rent, fee reserve and final wallet-budget checks are preserved.
- Settlement amount calculation cancels decimal scales before multiplication, avoiding intermediate overflow when the result fits in `u64`.
- Decode the on-chain `Closed` position status.
- Add optional `GlobalStats.usd` with separate notional and premium totals as decimal dollar strings, plus the number of priced confirmed trades. Older snapshots without USD values remain readable.
- Network examples load a Solana keypair file, support a separate maker owner, and complete the Hello → Welcome → StartAuth handshake. README includes setup and run commands.

- `QuoteExpiry::to_system_time` returns `Option<SystemTime>` and `From<QuoteExpiry> for SystemTime` is removed. Use `expiry.to_system_time().ok_or(...)` instead of an infallible conversion. Out-of-range wire timestamps retain their exact u64 value and no longer panic; `remaining_from` returns `None` for them.

- Derive and verify the RFQ market PDA before quote signing, including program, chain, option type and collateral/settlement mints. `RfqBinding::from_broadcast` uses the SDK's Acta program; `from_broadcast_for_program` accepts an explicitly configured deployment ID. Neither path trusts the endpoint to select the program.
- Validate the canonical maker authentication challenge before calling local or asynchronous signers, including reconnects and mid-session challenges. Raw-client examples use the same validator. Order signing bytes are unchanged.
- Decode referral counters as `u64`, matching the server and accepting its full range. Client code with explicit `i64` annotations must update those annotations.
- Ignore unknown subscription channels during recovery instead of sending `Unsubscribe("unknown")`. Unknown trading states still reject.

## 0.4.2 - 2026-09-10

- `managed_quote` sends with its recovered epoch, so stale quotes cannot cross a reconnect.
- Examples use maker-owner authentication and skip quotes beyond market expiry.

## 0.4.1 - 2026-09-10

- Shutdown drops queued data-lane frames even when the control lane is full; concurrent and cancelled `close()` waiters still await session shutdown.
- Typed indicative responses require Ready at enqueue and cannot cross a reconnect; public API and wire format are unchanged.

## 0.4.0 - 2026-08-31

- **Breaking:** `Welcome.server_time_unix_ms`, `AuthSuccess.expires_at`, `RfqBroadcast.sent_at_unix_ms` (server emission time), and `instruction_index` on all six known chain events are required; missing/null values reject. Timestamp units and protocol version are unchanged; credential expiry applies to resume, not socket lifetime. Chain-event identity is `(signature, instruction_index)`; only unknown variants lack an SDK identity.
- Quotes and cancellations share FIFO. `CancelQuoteAck` requires `request_id`, `rfq_id`, and `cancelled_order_ids`; lifecycle `QuoteCancelled` never resolves cancellation awaits. `CancelRfq` is send-only: `send_await` returns `NoCorrelationKey`, and `RfqClosed` is lifecycle evidence.
- Quote clients default to COD. `with_cancel_on_disconnect(true)` requests `cancel_on_disconnect`; absent `Welcome` support terminates with `FeatureUnsupported`, and `QuoteCancelReason::MakerDisconnected` decodes it. `GapPolicy::EndpointDefault` reconnects Quote sessions and surfaces Data gaps; explicit policies and `low_latency()` remain available. COD runs after server disconnect handling.
- CancelAll and COD remove retained non-winners in locked RFQs; removed orders cannot return through rollback, selected orders remain, and single-RFQ CancelQuote rejects locked RFQs.
- **Breaking:** `auto_reconcile` and `with_auto_reconcile` are removed. Recovery is required before Ready; commands submitted from Ready, including raw calls and queries, bind to its connection epoch. Typed data requests capture readiness and epoch together.
- Desired subscriptions, including removals and empty mint scopes, survive reconnect, resume, and fresh auth; rejected changes roll back. A second pending mutation returns `SubscriptionPending`; trading mutations before Ready or from a stale epoch return `NotReady`; subscription ACK timeouts start after write.
- `initial_subscribe` defines the desired target, not a wire request: recovery uses a new request ID and explicitly replaces both mint scopes, including empty scopes.
- Lifecycle `rfq_version`, `order_version`, and `QuoteCancelled.order_ids` are required; malformed values no longer default to zero or empty.
- `GetMyQuotes.scope` is required: unpaged `Live` is the complete owner set and paginated `History` is separate. `MakerQuoteInfo.state` is required: `Active { rank, rfq_version }`, `Retained { rfq_version }`, `AwaitingSignature { rfq_version }`, `Executing { rfq_version }`, or `Historical { status }`; missing fields reject and `Default` remains explicit Live.
- `MakerDataClient` adds typed `GetOrderStatus`: `Pending`, `Confirmed { position_pda }`, or `Unknown`; `Unknown` does not prove no execution. `ActiveRfqInfo.taker` is part of the `order_id` preimage, allowing reconnect recovery from `GetActiveRfqs`.
- A separate writer prioritizes control frames over the FIFO data lane while reads continue. Stalled writes fail their ticket within `write_timeout` and reconnect; full lanes return `QueueFull` from `send()`/`send_await()`.
- `ManagedWsConfig::low_latency()` uses 2 s ping/pong, 1 s writes, larger queues, and `GapPolicy::Reconnect`.
- **Breaking:** `ManagedWsConfig::new` takes `Arc<dyn SignerLike + Send + Sync>` instead of a challenge closure and pubkey; it defaults to the signer key, `with_auth_pubkey` sets a delegating owner, and `ChallengeSigner` is removed.
- `UnrenderablePositionInfo.created_at` is required. `QuoteBuilder::maker_owner()` and `ManagedWsConfig::with_auth_pubkey()` support delegated quote/auth keys without `order_id preimage mismatch`.
- A session-task panic publishes `Closed { reason: SessionPanicked }` and `Terminated`, then reaches `close()`. `SendTicket` is `#[must_use]`; dropping it before dequeue cancels its send.
- `RateLimitReason::TooManyFailedOrdersPerRfq` is recognized; `AuthRequiredAction::QueryQuotes` is removed.
- `GetMakerPositions` and `GetMyQuotes` add keyset `cursor`/`cursor_id` pagination from the prior row's `(created_at, pda/order_id)`, avoiding the 500-row server clamp. `MyQuotes` adds `has_more`; unrenderable positions add `created_at`.
- `invalid_signature` retries with backoff and becomes terminal after three consecutive rejections; `maker_not_registered`, `invalid_pubkey`, `blacklisted`, and `session_replaced` remain terminal. An authenticated reconnect resumes with the server session and reaches Ready in a new epoch.
- `QuoteRejected.reason` is `CapExceeded(CapError)` with `current`/`limit`; `cap_code` and `cap_detail` are removed. `RfqSkipped` retains `cap_detail`; `CapError` adds retryable `CapsUnavailable` and raw-payload `Unknown` fallback.
- MSRV is Rust 1.89. `valid_until` is `QuoteExpiry` with whole Unix seconds on `QuoteMessage`, `ReplaceQuoteMessage`, `QuoteReceivedMessage`, `MakerQuoteInfo`, and `QuoteRefreshRequested::min_valid_until`; the wire format is unchanged and `SystemTime` truncates to the signed second.
- `MakerResponse` owns its payload; `payload()` is infallible, `into_payload()` consumes it, and `subscribe_messages()` exposes whole messages. `RfqBinding` and `QuoteBuilder` derive signed preimages and wire quotes from one RFQ/strike set; unavailable strikes fail locally.
- All timestamp serialization matches backend truncation: `TimestampSeconds<i64>` no longer rounds 36 further fields, filters, or `GetMyTrades` cursors.
- `broadcast_buffer` is slot-bounded by parsed `Arc<ServerMessage>` values and can grow beyond the old 64-slot / 128 MiB envelope. `Hello`, `StartAuth`, and initial subscriptions use fallible write paths; unserializable subscriptions are validation errors.
- `QuoteMessage::into_replacement(old_order_id)` supports HSM/remote signers and replaces local-key-only `sign_replacing`. Closing interrupts authentication and readiness waits.
- Added `MakerQuoteClient`, typed `MakerDataClient` responses, and `Connected → Authenticated → Reconciled/Ready`. Managed sends serialize once and report socket-write completion; limits match 32 KiB messages and 50-quote batches.
- Added connect deadlines, reconnect limits/jitter, session resume, and terminal rejection/replacement handling. First subscribers receive a bounded since-spawn backlog; inbound payloads share `Arc<ServerMessage>`. Unknown messages/errors/nested enums remain connected, and future batch results retain status and `order_id` correlation.
- Managed sessions fairly serve inbound traffic under sustained sends, reject oversized initial subscriptions, and stop when the last handle drops.
- **Breaking:** `ChainClient` RPC helpers are async and signer references require `Sync`; configure initial subscriptions with `with_initial_subscribe()`; `ManagedMessageReceiver::recv()` returns owned `ManagedInbound`; default features are empty and Tokio multi-thread is optional; extensible public enums are non-exhaustive and require fallback match arms.

## 0.3.0 - 2026-07-15

- **Breaking:** WebSocket payloads move to `acta_maker_sdk::ws::types`; managed startup and inbound messages return typed configuration and stream errors.
- Managed WebSocket requests add reliable correlation, bounded queues, gap detection, explicit quote/data endpoints, and no quote replay after reconnect. Transport defaults use TCP_NODELAY, bounded buffers, and connect/auth/pong/write deadlines.
- Added verified async Ed25519 signing, safer nonce/keypair handling, stronger domain types, account/settlement validation, reserve-aware native SOL funding, referral/invite types, and `solana-client` 3.1.14.

## 0.2.0

- Replaced `get_maker_balances` / `MakerBalances` with `get_mm_summary` / `MmSummaryData` (`caps`, `positions`, `active_quotes`, `markets`, `tokens`, `maker_pda`, `computed_at`); `MakerBalanceCapInfo` adds `decimals`.
- `PositionUpdated` adds owner-only `caps_snapshot: MakerCapsSnapshot`; `AuthSuccessData` adds `maker_pda: Option<String>`.
- Maker position, quote, and trade rows add underlying/quote `mint`, `symbol`, and `decimals`; `MakerPositionInfo.status` is `PositionStatus` (`none | open | funded | liquidated | settled`) and adds `settlement_price`.

## 0.1.0

- Initial release: typed `WsClient`; auto-reconnecting/authenticating `ManagedWs` with `send_await()`; order preimages and Ed25519 signing; atomic nonces; optional chain instruction builders; hex/base58 wire utilities.
