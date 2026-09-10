# Changelog

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
