use std::future::Future;
use std::pin::Pin;

use ed25519_dalek::{Signature, VerifyingKey};
use thiserror::Error;

/// A signing operation whose implementation may wait for external I/O.
pub type SigningFuture<'a> =
    Pin<Box<dyn Future<Output = Result<[u8; 64], SigningError>> + Send + 'a>>;

/// Failure returned by an asynchronous signing provider.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SigningError {
    message: String,
}

impl SigningError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Asynchronous Ed25519 signer capability.
///
/// Implementations must not block the async executor while waiting for I/O.
pub trait AsyncSignerLike: Send + Sync {
    fn pubkey_bytes(&self) -> [u8; 32];

    fn sign_message<'a>(&'a self, message: &'a [u8]) -> SigningFuture<'a>;
}

pub(crate) async fn sign_verified<S: AsyncSignerLike + ?Sized>(
    signer: &S,
    message: &[u8],
) -> Result<[u8; 64], SigningError> {
    let signature = signer.sign_message(message).await?;
    let verifying_key = VerifyingKey::from_bytes(&signer.pubkey_bytes())
        .map_err(|_| SigningError::new("async signer returned an invalid Ed25519 public key"))?;
    verifying_key
        .verify_strict(message, &Signature::from_bytes(&signature))
        .map_err(|_| {
            SigningError::new("async signer returned a signature for a different key or message")
        })?;
    Ok(signature)
}
