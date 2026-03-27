# Changelog

## 0.1.0

Initial release.

- WebSocket client (`WsClient`) with typed messages
- Managed connection (`ManagedWs`) with auto-reconnect, auto-auth, `send_await()`
- Order preimage construction and Ed25519 signing (`compute_order_id`, `SignerLike`)
- Atomic nonce generator for concurrent quoting
- Solana instruction builders (optional `chain` feature)
- Wire encoding utilities (hex, base58)
