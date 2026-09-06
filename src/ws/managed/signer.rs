use std::sync::Arc;

use crate::orders::SignerLike;
use crate::signing::{AsyncSignerLike, SigningError, sign_verified};
use crate::wire::encode_base58;
use crate::ws::types::HelloData;

use super::ManagedWsConfig;

#[derive(Clone)]
pub(crate) enum ChallengeSigning {
    Local(Arc<dyn SignerLike + Send + Sync>),
    Async(Arc<dyn AsyncSignerLike>),
}

impl ChallengeSigning {
    pub(crate) async fn sign(&self, challenge: &str) -> Result<String, SigningError> {
        match self {
            Self::Local(signer) => Ok(signer.sign_message_base58(challenge.as_bytes())),
            Self::Async(signer) => sign_verified(signer.as_ref(), challenge.as_bytes())
                .await
                .map(|signature| encode_base58(&signature)),
        }
    }
}

impl ManagedWsConfig {
    /// Configure an asynchronous Ed25519 signer.
    #[must_use]
    pub fn new_async(
        url: impl Into<String>,
        hello: HelloData,
        signer: Arc<dyn AsyncSignerLike>,
    ) -> Self {
        let auth_pubkey = encode_base58(&signer.pubkey_bytes());
        Self::new_with_signing(url, hello, auth_pubkey, ChallengeSigning::Async(signer))
    }
}
