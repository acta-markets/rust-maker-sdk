//! SDK-owned domain model aligned with the backend contract.
//!
//! This module contains domain values and shared payload types. WebSocket
//! envelopes (`ClientMessage` and `ServerMessage`) belong to [`crate::ws::types`]
//! and must not be duplicated here. The SDK intentionally has no dependency on
//! the backend's internal types crate.

/// Fixed-point price scale shared with the backend and on-chain contract.
pub const PRICE_SCALE: u64 = 1_000_000_000;

#[macro_use]
mod macros;

pub mod domain;
pub mod errors;
pub mod ids;
pub mod invite;

pub use domain::*;
pub use errors::*;
pub use ids::*;
pub use invite::*;
