use std::sync::Arc;
use std::time::Duration;

use crate::orders::SignerLike;
use crate::ws::managed::{ChallengeSigning, ManagedWsConfig};
use crate::ws::types::HelloData;

#[derive(Clone)]
pub struct VaultOperatorConfig {
    pub managed: ManagedWsConfig,
    pub request_timeout: Duration,
    pub(super) vault_pda: String,
    pub(super) operator: Arc<dyn SignerLike + Send + Sync>,
}

impl VaultOperatorConfig {
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        hello: HelloData,
        vault_pda: impl Into<String>,
        operator: Arc<dyn SignerLike + Send + Sync>,
    ) -> Self {
        let vault_pda = vault_pda.into();
        let managed = ManagedWsConfig::new_with_signing(
            url,
            hello,
            operator.pubkey_base58(),
            ChallengeSigning::Local(Arc::clone(&operator)),
        )
        .with_vault_auth(vault_pda.clone());
        Self {
            managed,
            request_timeout: Duration::from_secs(5),
            vault_pda,
            operator,
        }
    }

    #[must_use]
    pub fn with_managed(mut self, adjust: impl FnOnce(ManagedWsConfig) -> ManagedWsConfig) -> Self {
        self.managed = adjust(self.managed);
        self
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    #[must_use]
    pub fn vault_pda(&self) -> &str {
        &self.vault_pda
    }

    #[must_use]
    pub fn operator_pubkey(&self) -> String {
        self.operator.pubkey_base58()
    }
}
