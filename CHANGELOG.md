# Changelog

## 0.4.1 - 2026-09-10

- Stop queued data-lane frames from being written during shutdown when the control lane is full.
- Keep `close().await` waiting for session shutdown across concurrent callers and cancellation of an earlier close future.
- Reject indicative price responses before Ready so a reconnect between the typed client's readiness check and enqueue cannot send an unbound response on the next connection.
- Add regression tests for saturated queues, concurrent close calls and cancelled close futures. Public API and wire format are unchanged.
- Handle interleaved heartbeats in the pending-subscription cancellation test.

## 0.4.0 - 2026-08-31

- Breaking: `Welcome.server_time_unix_ms`, `AuthSuccess.expires_at`, `RfqBroadcast.sent_at_unix_ms` and `instruction_index` on all six known chain events are required. Missing/null values are rejected; existing timestamp units and protocol version are unchanged. Credential expiry applies to resume, not socket lifetime.

- Quotes and cancellations share FIFO order; cancellation ACKs report the applied server result. `CancelQuoteAck` requires `request_id`, `rfq_id` and `cancelled_order_ids`; lifecycle `QuoteCancelled` no longer resolves cancellation awaits.
- Managed Quote clients enable COD by default. `GapPolicy::EndpointDefault` reconnects Quote sessions and only surfaces gaps on Data sessions; explicit policies and the low-latency preset remain available. COD applies after the server processes disconnect and leaves locked/in-flight orders intact.
- Backend CancelAll and COD also remove retained non-winning quotes in locked RFQs, so removed orders cannot return after rollback. The selected order remains intact; single-RFQ CancelQuote still rejects locked RFQs.
- Removed `auto_reconcile` and `with_auto_reconcile`: managed recovery is mandatory before Ready. All commands submitted from Ready, including queries and raw managed calls, are bound to that connection epoch and cannot cross reconnect. Typed data requests capture readiness and epoch together, so a state change during request conversion cannot enqueue an unbound query.
- Dynamic subscription changes survive reconnect, resume and fresh authentication. Explicit server rejection rolls back the pending change.
- A second unfinished subscription mutation returns `SubscriptionPending`; trading mutations submitted before Ready or from an older connection epoch return `NotReady`. The subscription ACK timeout starts after successful socket write.
- `initial_subscribe` defines the desired target, not a wire request: its `request_id` is replaced by a fresh recovery request ID. Recovery explicitly replaces both server mint scopes, including clearing a persisted scope when the desired set is empty; omitted initial filters do not mean inherit the server scope.
- Lifecycle `rfq_version`, `order_version` and `QuoteCancelled.order_ids` are required on the wire; malformed frames no longer silently default to zero or an empty set.
- `GetMyQuotes` now requires a wire `scope`: unpaged `Live` is the complete owner set, while paginated `History` is a separate projection. `MakerQuoteInfo.state` is a required ADT: `Active { rank, rfq_version }`, `Retained { rfq_version }`, `AwaitingSignature { rfq_version }`, `Executing { rfq_version }`, or `Historical { status }`. Missing wire fields fail decoding; Rust `Default` remains an ergonomic constructor for an explicit Live request.
- `GetOrderStatus` now has the same typed `MakerDataClient` facade as the other maker reads. Its required state is `Pending`, `Confirmed { position_pda }`, or `Unknown`; `Unknown` is not a no-execution result.
- `CancelRfq` is send-only: `send_await` returns `NoCorrelationKey` without sending. `RfqClosed` is lifecycle evidence, not a correlated command receipt.
- Added matching contract/SDK order preimage and digest fixtures.

- Added `ActiveRfqInfo.taker`: the RFQ taker is an `order_id` preimage input, so an RFQ still open after a reconnect can be quoted from the `GetActiveRfqs` re-read.
- Added cancel-on-disconnect: `ManagedWsConfig::with_cancel_on_disconnect(true)` requests the `cancel_on_disconnect` server feature, and the session terminates with `FeatureUnsupported` if `Welcome` does not enable it. `QuoteCancelReason::MakerDisconnected` decodes the matching cancellation.
- The session task reads and correlates while a separate writer drains control traffic ahead of the FIFO data lane shared by quotes, cancellations and queries. A stalled write no longer blocks reading; it fails its ticket within `write_timeout` and the session reconnects. A full lane returns `QueueFull` from `send()`/`send_await()` instead of waiting.
- Added `ManagedWsConfig::low_latency()`: 2 s ping/pong, 1 s write timeout, larger queues and `GapPolicy::Reconnect`.
- Breaking: `ManagedWsConfig::new` takes an `Arc<dyn SignerLike + Send + Sync>` instead of a challenge closure and a pubkey; the pubkey defaults to the signer's key and `with_auth_pubkey` names a delegating owner. `ChallengeSigner` is gone.
- Fixed: `UnrenderablePositionInfo.created_at` is required, as the server always sends it.
- Added `examples/loopback_latency.rs` and a README latency budget.

- Fixed: the typed quote path hard-wired the order preimage's maker to the signing key, so every quote signed by a delegated quote key was rejected as `order_id preimage mismatch`. `QuoteBuilder::maker_owner()` sets the identity the server reconstructs; `ManagedWsConfig::with_auth_pubkey()` does the same for `new_async`, whose auth identity was pinned to the signer.
- Fixed: a panic in the session task left `state()` at `Ready` and message subscribers waiting forever, because the handle keeps the broadcast channels alive. The panic now publishes `Closed { reason: SessionPanicked }` and a `Terminated` event, then propagates to `close()` as before.
- `SendTicket` is `#[must_use]` and documents that dropping it before the session task dequeues the command cancels the send. Fire-and-forget `try_send` loses quotes; hold the ticket.
- Added `RateLimitReason::TooManyFailedOrdersPerRfq`, which the server sends; it decoded as `Unknown`. Removed `AuthRequiredAction::QueryQuotes`, which the server never emits.
- Added keyset pagination to `GetMakerPositions` and `GetMyQuotes`: optional `cursor`/`cursor_id` request fields, filled from the last row of the previous page (`created_at` plus position `pda` or quote `order_id`). Listings used to truncate at the server's 500-row clamp with `has_more` and no way to fetch the rest. `MyQuotes` now carries `has_more` like the other listings, and unrenderable positions carry `created_at`, so a page may end on one.
- A single `invalid_signature` rejection no longer terminates the managed session: it retries with the normal backoff and goes terminal on the third rejection with no successful authentication in between, so a one-off signer or server glitch cannot permanently stop an unattended maker. Structural rejections (`maker_not_registered`, `invalid_pubkey`, `blacklisted`, `session_replaced`) stay immediately terminal.
- `QuoteRejected.reason` is `CapExceeded(CapError)`: the failing dimension with its `current`/`limit` rides inside the reason (`{"cap_exceeded": …}` on the wire), so a cap rejection cannot arrive without its detail; the separate `cap_code`/`cap_detail` fields are gone. `RfqSkipped` keeps `cap_detail`. `CapError` gains `CapsUnavailable` (venue-wide pause — retry, do not de-risk) and a raw-payload `Unknown` fallback, so a cap variant the SDK predates parses instead of dropping the connection.
- The reconnect loop is covered end to end: a dropped authenticated session reconnects, resumes with the server-issued session ID, and reaches `Ready` on the next connection epoch.
- Declared `rust-version = "1.89"`. The protocol drift check now compares fourteen wire enums at variant level rather than two at tag level, and CI runs the fmt/clippy/test/doc/package pipeline on stable and an MSRV check on 1.89.
- README: the quick start now subscribes to the `Rfqs` channel (a session that never subscribes hears no RFQs), and new sections cover onboarding preconditions, delegated signing keys, price/strike units, re-hydration after a quote-plane reconnect, and who runs settlement.
- Fixed: a quote whose `valid_until` carried a sub-second fraction was signed for one second and transmitted for another, because the order preimage truncates to whole seconds while the JSON timestamp rounded. Affected quotes were rejected as `order_id preimage mismatch`, roughly half the time and depending only on when the expiry was computed.
- Added `RfqBroadcast.sent_at_unix_ms`: when the server emitted the broadcast, so a maker can tell a promptly delivered RFQ from one delayed behind a slow fan-out. This timestamp is required.
- `MakerResponse` owns its payload rather than re-projecting it out of a retained `Arc<ServerMessage>`, so `payload()` cannot fail; `into_payload()` takes the value. Use `subscribe_messages()` when a whole server message is wanted.
- Added `RfqBinding` and `QuoteBuilder`: an RFQ's keys are decoded once, its strike set is resolved, and the signed preimage and the wire message are derived from the same values, so they cannot disagree. A strike the RFQ did not offer is refused locally instead of after a round trip.
- Fixed: every wire timestamp now uses the backend's encoding. The SDK wrote `TimestampSeconds<i64>`, which rounds, against a backend that truncates — the same split that broke signed quote expiries, present on 36 further fields. Client-sent filters and the `GetMyTrades` pagination cursor could land a second away from the value the server read.
- Fixed: `broadcast_buffer` could not be raised at all. Its envelope was priced at `capacity × transport.max_message_size`, which put the default of 64 exactly on the 128 MiB ceiling, so the only defence against `ManagedReceiveError::Gap` was pinned at its floor. The ring holds parsed `Arc<ServerMessage>` handles rather than wire frames, so it is now bounded by slot count.
- `ManagedWsConfig` no longer pre-serializes `Hello`, `StartAuth` and the initial subscribe. Each is sent through the normal fallible write path, removing three panics from the config builder; a subscribe that cannot be serialized is now a validation error.
- A maker whose key lives in an HSM or a remote signing service can now build replacements: `QuoteMessage::into_replacement(old_order_id)` replaces the local-key-only `sign_replacing`, since a replacement is the same signed quote plus the order it supersedes.
- Both maker client surfaces are pinned by a test, so a dropped or renamed method is a compile error rather than a silent removal.
- Authentication and the readiness barrier race their deadline against shutdown through one named helper; closing during either phase is now covered by tests.
- A control frame that ends a session attempt is modelled as `Option<ControlFailure>` instead of a `Continue` variant that callers had to rule out, removing an `unreachable!` from authentication.
- Breaking: `valid_until` is a `QuoteExpiry` of whole Unix seconds on `QuoteMessage`, `ReplaceQuoteMessage`, `QuoteReceivedMessage`, `MakerQuoteInfo`, and `QuoteRefreshRequested::min_valid_until`. The wire format is unchanged; conversion from `SystemTime` truncates, matching the preimage.
- Added `MakerQuoteClient`, typed `MakerDataClient` responses, and the `Connected → Authenticated → Reconciled/Ready` state model.
- Matched the server's 32 KiB message and 50-quote batch limits. Managed sends serialize once and report socket-write completion.
- Added connect deadlines, reconnect limits, jitter, session resume, and terminal handling for rejected or replaced sessions.
- The first subscriber receives the bounded since-spawn backlog. Inbound payloads share one `Arc<ServerMessage>`.
- Unknown message types, error payloads, and nested enum values no longer disconnect managed clients.
- Future batch-result payloads retain their status and `order_id`, so batch requests still correlate without a timeout.
- Managed sessions fairly service inbound traffic under sustained sends, reject oversized initial subscriptions, and stop when the last handle is dropped.
- All six known chain events require `instruction_index`; `(signature, instruction_index)` is their identity. Only an unknown event variant has no SDK identity.
- Breaking: `ChainClient` RPC helpers are async, and signer references passed to them must implement `Sync`.
- Breaking: configure the initial subscription with `ManagedWsConfig::with_initial_subscribe()`; the backing field is no longer public.
- Breaking: `ManagedMessageReceiver::recv()` returns an owned `ManagedInbound` instead of `Arc<ManagedInbound>`.
- Default features are empty; Tokio's multi-thread runtime is no longer required.
- Breaking: extensible public enums are non-exhaustive and require a fallback match arm.

## 0.3.0 - 2026-07-15

- Breaking: WebSocket payloads move to `acta_maker_sdk::ws::types`; managed startup and inbound
  messages now return typed configuration and stream errors.
- Managed WebSocket requests have reliable correlation, bounded queues, gap detection, explicit
  quote/data endpoints, and no automatic quote replay after reconnect.
- Transport defaults now use TCP_NODELAY, bounded buffers, and connect/auth/pong/write deadlines.
- Added verified async Ed25519 signing, safer nonce and keypair handling, and stronger domain types.
- Chain RPC helpers now validate accounts and settlement inputs; native SOL funding uses explicit
  reserve-aware configuration.
- Added referral/invite protocol types and updated `solana-client` to 3.1.14.

## 0.2.0

- Replaced `get_maker_balances` / `MakerBalances` with `get_mm_summary` / `MmSummaryData` (`caps`, `positions`, `active_quotes`, `markets`, `tokens`, `maker_pda`, `computed_at`). `MakerBalanceCapInfo` gains `decimals`.
- `PositionUpdated` now carries `caps_snapshot: MakerCapsSnapshot` (owner-only), so no follow-up `GetMyCaps` is needed.
- `AuthSuccessData` gains `maker_pda: Option<String>`.
- `MakerPositionInfo` / `MakerQuoteInfo` / `MakerTradeInfo` now carry underlying/quote `mint/symbol/decimals`; `MakerPositionInfo.status` is `PositionStatus` enum (`none | open | funded | liquidated | settled`) and gains `settlement_price`.

## 0.1.0

Initial release.

- WebSocket client (`WsClient`) with typed messages
- Managed connection (`ManagedWs`) with auto-reconnect, auto-auth, `send_await()`
- Order preimage construction and Ed25519 signing (`compute_order_id`, `SignerLike`)
- Atomic nonce generator for concurrent quoting
- Solana instruction builders (optional `chain` feature)
- Wire encoding utilities (hex, base58)
