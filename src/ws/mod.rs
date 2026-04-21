#[cfg(feature = "ws-client")]
pub mod client;

#[cfg(feature = "ws-client")]
pub mod error;
#[cfg(feature = "ws-client")]
pub mod managed;
pub mod reconnect;
pub mod types;
