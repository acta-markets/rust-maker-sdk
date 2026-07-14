# Changelog

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
- `PositionUpdated` now carries `caps_snapshot: MakerCapsSnapshot` (owner-only) — no follow-up `GetMyCaps` needed.
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
