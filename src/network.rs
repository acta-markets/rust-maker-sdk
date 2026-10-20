//! Public Acta API origins. Append the WebSocket path (`/maker`,
//! `/maker/data`, `/vault`) or pass the origin to `normalize_maker_ws_url`.

/// Devnet HTTP API origin.
pub const DEVNET_API: &str = "https://devnet-api.acta.markets";
/// Devnet WebSocket origin.
pub const DEVNET_WS: &str = "wss://devnet-api.acta.markets";
/// Mainnet-beta HTTP API origin.
pub const MAINNET_API: &str = "https://beta-api.acta.markets";
/// Mainnet-beta WebSocket origin.
pub const MAINNET_WS: &str = "wss://beta-api.acta.markets";
