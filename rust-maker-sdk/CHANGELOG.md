# Changelog

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
