//! Read-only client for the public Acta HTTP API.

mod types;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_NONE_MATCH};
use serde::de::DeserializeOwned;

use types::{MarketsResponse, TokensResponse, VaultPositionsResponse, VaultsResponse};
pub use types::{
    PremiumFlow, ProjectionCursor, TokenProgramKind, Vault, VaultCapacity, VaultExposure,
    VaultExposurePosition, VaultExposureTotal, VaultFees, VaultLeg, VaultMetadata,
    VaultPerformance, VaultPolicy, VaultPosition, VaultPositionRole, VaultPositionStatus,
    VaultPositionType, VaultPremium, VaultReturns, VaultStaging, Venue, WindowReturn,
};

use crate::ws::types::{MarketInfo, TokenInfo};

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
    #[error("invalid response body: {0}")]
    Decode(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketFilter {
    pub underlying: Option<String>,
    pub quote: Option<String>,
    pub expiry: Option<u64>,
    pub is_put: Option<bool>,
    pub active: Option<bool>,
}

impl MarketFilter {
    fn query(&self) -> String {
        let mut params = Vec::new();
        if let Some(underlying) = &self.underlying {
            params.push(format!("underlying={underlying}"));
        }
        if let Some(quote) = &self.quote {
            params.push(format!("quote={quote}"));
        }
        if let Some(expiry) = self.expiry {
            params.push(format!("expiry={expiry}"));
        }
        if let Some(is_put) = self.is_put {
            params.push(format!("is_put={is_put}"));
        }
        if let Some(active) = self.active {
            params.push(format!("active={active}"));
        }
        if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    base_url: String,
    cache: Arc<Mutex<HashMap<String, Cached>>>,
}

#[derive(Debug)]
struct Cached {
    etag: String,
    body: Arc<[u8]>,
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
        Self {
            client,
            base_url,
            cache: Arc::default(),
        }
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn vaults(&self) -> Result<Vec<Vault>, HttpError> {
        let response: VaultsResponse = self.get("/api/v1/vaults").await?;
        Ok(response.vaults)
    }

    /// Vaults whose primary delegate is `delegate` with an unexpired grant, the
    /// same rule `/vault` authentication applies.
    pub async fn delegated_vaults(&self, delegate: &str) -> Result<Vec<Vault>, HttpError> {
        let _ = decode_base58_32(delegate)?;
        let response: VaultsResponse = self
            .get(&format!("/api/v1/vaults?delegate={delegate}"))
            .await?;
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

    /// Open and funded positions with indicative spot, moneyness and intrinsic value.
    pub async fn vault_exposure(&self, pda: &str) -> Result<VaultExposure, HttpError> {
        let _ = decode_base58_32(pda)?;
        self.get(&format!("/api/v1/vaults/{pda}/exposure")).await
    }

    pub async fn markets(&self, filter: &MarketFilter) -> Result<Vec<MarketInfo>, HttpError> {
        let response: MarketsResponse = self
            .get(&format!("/api/v1/markets{}", filter.query()))
            .await?;
        Ok(response.markets)
    }

    pub async fn tokens(&self) -> Result<BTreeMap<String, TokenInfo>, HttpError> {
        let response: TokensResponse = self.get("/api/v1/tokens").await?;
        Ok(response
            .tokens
            .into_iter()
            .map(|token| (token.mint.clone(), token))
            .collect())
    }

    pub async fn venue(&self) -> Result<Venue, HttpError> {
        self.get("/api/v1/venue").await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpError> {
        let mut request = self.client.get(format!("{}{path}", self.base_url));
        if let Some(etag) = self.cached(path, |cached| cached.etag.clone()) {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let response = request.send().await?;
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED
            && let Some(body) = self.cached(path, |cached| Arc::clone(&cached.body))
        {
            return Ok(serde_json::from_slice(&body)?);
        }
        if !status.is_success() {
            let body = response.text().await?;
            return Err(HttpError::Status {
                status: status.as_u16(),
                body,
            });
        }
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.bytes().await?;
        let value = serde_json::from_slice(&body)?;
        if let Some(etag) = etag {
            let _ = self.cache().insert(
                path.to_owned(),
                Cached {
                    etag,
                    body: Arc::from(body.as_ref()),
                },
            );
        }
        Ok(value)
    }

    fn cached<R>(&self, path: &str, read: impl FnOnce(&Cached) -> R) -> Option<R> {
        self.cache().get(path).map(read)
    }

    fn cache(&self) -> MutexGuard<'_, HashMap<String, Cached>> {
        match self.cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests;
