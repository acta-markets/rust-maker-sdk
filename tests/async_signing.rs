#![cfg(feature = "ws-client")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use acta_maker_sdk::ws::types::HelloData;
use acta_maker_sdk::{
    AsyncSignerLike, ManagedWsConfig, SigningError, SigningFuture, sign_order_id_with_async_signer,
};
use ed25519_dalek::{Signer, SigningKey};

struct RemoteSigner {
    secret: [u8; 32],
    calls: Arc<AtomicUsize>,
    fail: bool,
    corrupt: bool,
}

impl AsyncSignerLike for RemoteSigner {
    fn pubkey_bytes(&self) -> [u8; 32] {
        SigningKey::from_bytes(&self.secret)
            .verifying_key()
            .to_bytes()
    }

    fn sign_message<'a>(&'a self, message: &'a [u8]) -> SigningFuture<'a> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return Err(SigningError::new("signer unavailable"));
            }
            let mut signature = SigningKey::from_bytes(&self.secret)
                .sign(message)
                .to_bytes();
            if self.corrupt {
                signature[0] ^= 1;
            }
            Ok(signature)
        })
    }
}

#[tokio::test]
async fn async_order_signer_is_awaited_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let signer = RemoteSigner {
        secret: [7; 32],
        calls: Arc::clone(&calls),
        fail: false,
        corrupt: false,
    };
    let order_id = [9_u8; 32];

    let signature = sign_order_id_with_async_signer(&order_id, &signer)
        .await
        .expect("remote signer should succeed");

    assert_ne!(signature, [0; 64]);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn async_order_signer_error_is_preserved() {
    let signer = RemoteSigner {
        secret: [7; 32],
        calls: Arc::new(AtomicUsize::new(0)),
        fail: true,
        corrupt: false,
    };

    let error = sign_order_id_with_async_signer(&[9; 32], &signer)
        .await
        .expect_err("provider failure must be visible");

    assert_eq!(error.to_string(), "signer unavailable");
}

#[tokio::test]
async fn async_order_signer_rejects_an_invalid_signature() {
    let signer = RemoteSigner {
        secret: [7; 32],
        calls: Arc::new(AtomicUsize::new(0)),
        fail: false,
        corrupt: true,
    };

    let error = sign_order_id_with_async_signer(&[9; 32], &signer)
        .await
        .expect_err("an invalid signature must not leave the SDK");

    assert_eq!(
        error.to_string(),
        "async signer returned a signature for a different key or message"
    );
}

#[test]
fn managed_config_accepts_remote_signer_capability() {
    let signer = RemoteSigner {
        secret: [7; 32],
        calls: Arc::new(AtomicUsize::new(0)),
        fail: false,
        corrupt: false,
    };
    let config = ManagedWsConfig::new_async(
        "wss://example.test",
        HelloData {
            protocol_version: "1".to_string(),
            features: Vec::new(),
            client_name: None,
            client_version: None,
        },
        Arc::new(signer),
    );

    assert!(config.validate().is_ok());
}
