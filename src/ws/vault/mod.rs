mod client;
mod config;
mod projection;
mod tx;

pub use client::{VaultClientError, VaultOperatorClient, VaultRequestError};
pub use config::VaultOperatorConfig;
pub use projection::{VaultProjection, VaultProjectionReceiver};
pub use tx::{
    VaultTxError, ensure_deadline_ahead, sign_transaction_base64,
    sign_transaction_base64_unverified,
};

#[cfg(test)]
#[path = "../vault_tests.rs"]
mod tests;
