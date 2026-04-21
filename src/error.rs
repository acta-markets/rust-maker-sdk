use thiserror::Error;

use crate::nonce::NonceError;
use crate::orders::OrderError;
use crate::wire::WireError;

#[derive(Debug, Error)]
pub enum ActaSdkError {
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error(transparent)]
    Order(#[from] OrderError),
    #[error(transparent)]
    Nonce(#[from] NonceError),
    #[cfg(feature = "ws-client")]
    #[error(transparent)]
    Ws(#[from] crate::ws::error::WsClientError),
    #[cfg(feature = "ws-client")]
    #[error(transparent)]
    ManagedWs(#[from] crate::ws::managed::ManagedWsError),
    #[cfg(feature = "chain")]
    #[error(transparent)]
    Chain(#[from] crate::chain::ix::ChainIxError),
    #[cfg(feature = "chain-rpc")]
    #[error(transparent)]
    ChainRpc(#[from] crate::chain::rpc::ChainError),
}
