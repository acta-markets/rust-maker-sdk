//! Maker quote and data clients.

mod data;
mod quote;

pub use data::{MakerDataClient, MakerDataRequest, MakerDataRequestError, MakerResponse};
pub use quote::MakerQuoteClient;

use crate::ws::managed::{MakerWsEndpoint, ManagedWsConfigError};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MakerClientError {
    #[error("{client} requires the {required:?} endpoint, got {actual:?}")]
    WrongEndpoint {
        client: &'static str,
        required: MakerWsEndpoint,
        actual: MakerWsEndpoint,
    },
    #[error(transparent)]
    InvalidConfig(#[from] ManagedWsConfigError),
}
