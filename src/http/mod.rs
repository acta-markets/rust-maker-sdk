//! Read-only client for the public Acta HTTP API.

mod types;

use std::time::Duration;

use serde::de::DeserializeOwned;

pub use types::{
    Market, ProjectionCursor, TokenProgramKind, TrackRecord, Vault, VaultPerformance, VaultPolicy,
    VaultPosition, VaultPositionRole, VaultPositionStatus, VaultPositionType,
};
use types::{MarketsResponse, VaultPositionsResponse, VaultsResponse};

use crate::wire::{WireError, decode_base58_32};

const TIMEOUT: Duration = Duration::from_secs(10);
const POSITION_PAGE: u16 = 200;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HttpError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("invalid address: {0}")]
    Address(#[from] WireError),
}

#[derive(Debug, Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    base_url: String,
}

impl HttpClient {
    /// `base_url` is the API origin, e.g. `https://beta-api.acta.markets`.
    pub fn new(base_url: impl Into<String>) -> Result<Self, HttpError> {
        let client = reqwest::Client::builder().timeout(TIMEOUT).build()?;
        Ok(Self::with_client(client, base_url))
    }

    #[must_use]
    pub fn with_client(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            let _ = base_url.pop();
        }
        Self { client, base_url }
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn vaults(&self) -> Result<Vec<Vault>, HttpError> {
        let response: VaultsResponse = self.get("/api/v1/vaults").await?;
        Ok(response.vaults)
    }

    pub async fn vault(&self, pda: &str) -> Result<Vault, HttpError> {
        let _ = decode_base58_32(pda)?;
        self.get(&format!("/api/v1/vaults/{pda}")).await
    }

    /// Every unsettled position of the vault, following the server cursor.
    pub async fn vault_positions(&self, pda: &str) -> Result<Vec<VaultPosition>, HttpError> {
        let _ = decode_base58_32(pda)?;
        let base = format!("/api/v1/vaults/{pda}/positions?settled=false&limit={POSITION_PAGE}");
        let mut positions = Vec::new();
        let mut path = base.clone();
        loop {
            let page: VaultPositionsResponse = self.get(&path).await?;
            positions.extend(page.positions);
            match page.next_cursor {
                Some(cursor) => path = format!("{base}&cursor={cursor}"),
                None => return Ok(positions),
            }
        }
    }

    pub async fn markets(&self) -> Result<Vec<Market>, HttpError> {
        let response: MarketsResponse = self.get("/api/v1/markets").await?;
        Ok(response.markets)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpError> {
        let response = self
            .client
            .get(format!("{}{path}", self.base_url))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            return Err(HttpError::Status {
                status: status.as_u16(),
                body,
            });
        }
        Ok(response.json().await?)
    }
}

#[cfg(test)]
mod tests;
