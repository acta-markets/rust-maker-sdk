//! Domain values shared by WebSocket and chain APIs.

/// Fixed-point price scale shared with the backend and on-chain contract.
pub const PRICE_SCALE: u64 = 1_000_000_000;

#[macro_use]
mod macros;

pub mod address;
pub mod amounts;
pub use address::{Address32, AddressError};
#[cfg(feature = "decimal")]
mod decimal;
pub mod domain;
pub mod errors;
pub mod expiry;
pub mod ids;
pub mod invite;
pub mod products;
pub use products::{InvalidProductSet, ProductSet};
pub mod unix_time;
pub mod vault;

pub use amounts::{PhysicalPremium, physical_premium, total_premium_quote_atoms};
pub use domain::*;
pub use errors::*;
pub use expiry::QuoteExpiry;
pub use ids::*;
pub use invite::*;
pub use unix_time::{UnixMillis, UnixSeconds};
pub use vault::*;
