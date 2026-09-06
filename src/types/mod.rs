//! Domain values shared by WebSocket and chain APIs.

/// Fixed-point price scale shared with the backend and on-chain contract.
pub const PRICE_SCALE: u64 = 1_000_000_000;

#[macro_use]
mod macros;

pub mod domain;
pub mod errors;
pub mod expiry;
pub mod ids;
pub mod invite;
pub mod unix_time;

pub use domain::*;
pub use errors::*;
pub use expiry::QuoteExpiry;
pub use ids::*;
pub use invite::*;
pub use unix_time::{UnixMillis, UnixSeconds};
