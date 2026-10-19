mod client;
mod config;
#[cfg(feature = "http")]
mod fleet;
mod gate;
mod projection;
#[cfg(feature = "chain-rpc")]
mod sell;
mod tx;

pub use client::{VaultClientError, VaultOperatorClient, VaultRequestError};
pub use config::VaultOperatorConfig;
#[cfg(feature = "http")]
pub use fleet::{FleetListing, VaultFleet, VaultFleetConfig};
pub use gate::{SendGate, Ungated};
pub use projection::{VaultProjection, VaultProjectionReceiver};
#[cfg(feature = "chain-rpc")]
pub use sell::{SaleDesk, SaleError, SaleOutcome, SaleStage, SignedSale, VaultSale};
pub use tx::VaultTxError;

#[cfg(test)]
#[path = "../vault_tests.rs"]
mod tests;
