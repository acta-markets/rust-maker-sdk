//! Read-only client for the public Acta HTTP API.

mod types;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_NONE_MATCH};
use serde::de::DeserializeOwned;

pub use types::{
    CountLimit, PremiumFlow, ProjectionCursor, TokenProgramKind, Vault, VaultCapacity, VaultCycle,
    VaultDepositRequest, VaultExposure, VaultExposurePosition, VaultExposureTotal, VaultFees,
    VaultLeg, VaultMetadata, VaultPerformance, VaultPolicy, VaultPosition, VaultPositionProduct,
    VaultPositionRole, VaultPositionStatus, VaultPositionType, VaultPremium, VaultReturns,
    VaultStaging, VaultWithdrawRequest, Venue, WindowReturn,
};
use types::{MarketsResponse, TokensResponse, VaultPositionsResponse, VaultsResponse};
use types::{VaultCyclesResponse, VaultRequestsResponse};

use crate::ws::types::{MarketInfo, TokenInfo};

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HttpError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("invalid response body: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("page cursor did not advance")]
    InvalidCursor,
    #[error(transparent)]
    InvalidPage(#[from] crate::paging::InvalidPage),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketFilter {
    pub underlying: Option<crate::Address32>,
    pub quote: Option<crate::Address32>,
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
        static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let client = match CLIENT.get() {
            Some(client) => client.clone(),
            None => {
                let client = reqwest::Client::builder().timeout(TIMEOUT).build()?;
                CLIENT.get_or_init(|| client).clone()
            }
        };
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
        let response: VaultsResponse = self
            .collect(
                "/api/v1/vaults",
                |all: &VaultsResponse| all.next_cursor.as_deref(),
                |all, page| {
                    all.vaults.extend(page.vaults);
                    all.next_cursor = page.next_cursor;
                },
            )
            .await?;
        Ok(response.vaults)
    }

    /// Vaults whose primary delegate is `delegate` with an unexpired grant, the
    /// same rule `/vault` authentication applies.
    pub async fn delegated_vaults(
        &self,
        delegate: crate::Address32,
    ) -> Result<Vec<Vault>, HttpError> {
        let response: VaultsResponse = self
            .collect(
                &format!("/api/v1/vaults?delegate={delegate}"),
                |all: &VaultsResponse| all.next_cursor.as_deref(),
                |all, page| {
                    all.vaults.extend(page.vaults);
                    all.next_cursor = page.next_cursor;
                },
            )
            .await?;
        Ok(response.vaults)
    }

    pub async fn vault(&self, pda: crate::Address32) -> Result<Vault, HttpError> {
        self.get(&format!("/api/v1/vaults/{pda}")).await
    }

    /// Every unsettled position of the vault, following the server cursor.
    pub async fn vault_positions(
        &self,
        pda: crate::Address32,
    ) -> Result<Vec<VaultPosition>, HttpError> {
        let response: VaultPositionsResponse = self
            .collect(
                &format!("/api/v1/vaults/{pda}/positions?settled=false"),
                |all: &VaultPositionsResponse| all.next_cursor.as_deref(),
                |all, page| {
                    all.positions.extend(page.positions);
                    all.next_cursor = page.next_cursor;
                },
            )
            .await?;
        Ok(response.positions)
    }

    /// Every cycle of the vault, newest first.
    pub async fn vault_cycles(&self, pda: crate::Address32) -> Result<Vec<VaultCycle>, HttpError> {
        let response: VaultCyclesResponse = self
            .collect(
                &format!("/api/v1/vaults/{pda}/cycles"),
                |all: &VaultCyclesResponse| all.next_cursor.as_deref(),
                |all, page| {
                    all.cycles.extend(page.cycles);
                    all.next_cursor = page.next_cursor;
                },
            )
            .await?;
        Ok(response.cycles)
    }

    /// Every pending deposit of the vault, read without Solana RPC.
    pub async fn vault_deposit_requests(
        &self,
        pda: crate::Address32,
    ) -> Result<Vec<VaultDepositRequest>, HttpError> {
        self.vault_requests(&format!("/api/v1/vaults/{pda}/deposit-requests"))
            .await
    }

    /// Every pending withdrawal of the vault, read without Solana RPC.
    pub async fn vault_withdraw_requests(
        &self,
        pda: crate::Address32,
    ) -> Result<Vec<VaultWithdrawRequest>, HttpError> {
        self.vault_requests(&format!("/api/v1/vaults/{pda}/withdraw-requests"))
            .await
    }

    async fn vault_requests<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>, HttpError> {
        let response: VaultRequestsResponse<T> = crate::paging::fetch_all(
            |cursor| async move { self.get(&Self::page_path(path, 100, &cursor)?).await },
            |all: &VaultRequestsResponse<T>| all.next_cursor.as_deref(),
            |all, page| {
                all.requests.extend(page.requests);
                all.next_cursor = page.next_cursor;
            },
        )
        .await?;
        Ok(response.requests)
    }

    /// Open and funded positions with indicative spot, moneyness and intrinsic value.
    pub async fn vault_exposure(&self, pda: crate::Address32) -> Result<VaultExposure, HttpError> {
        self.collect(
            &format!("/api/v1/vaults/{pda}/exposure"),
            |all: &VaultExposure| all.next_cursor.as_deref(),
            |all, page| {
                all.positions.extend(page.positions);
                all.next_cursor = page.next_cursor;
            },
        )
        .await
    }

    /// One exposure page. The totals describe the full cached snapshot.
    pub async fn vault_exposure_page(
        &self,
        pda: crate::Address32,
        limit: u16,
        cursor: Option<&str>,
    ) -> Result<VaultExposure, HttpError> {
        let cursor = cursor.map_or(crate::ws::types::Cursor::Start {}, |id| {
            crate::ws::types::Cursor::After { id: id.to_owned() }
        });
        let path = Self::page_path(&format!("/api/v1/vaults/{pda}/exposure"), limit, &cursor)?;
        self.get(&path).await
    }

    pub async fn markets(&self, filter: &MarketFilter) -> Result<Vec<MarketInfo>, HttpError> {
        let response: MarketsResponse = self
            .collect(
                &format!("/api/v1/markets{}", filter.query()),
                |all: &MarketsResponse| all.next_cursor.as_deref(),
                |all, page| {
                    all.markets.extend(page.markets);
                    all.next_cursor = page.next_cursor;
                },
            )
            .await?;
        Ok(response.markets)
    }

    pub async fn tokens(&self) -> Result<BTreeMap<crate::Address32, TokenInfo>, HttpError> {
        let response: TokensResponse = self
            .collect(
                "/api/v1/tokens",
                |all: &TokensResponse| all.next_cursor.as_deref(),
                |all, page| {
                    all.tokens.extend(page.tokens);
                    all.next_cursor = page.next_cursor;
                },
            )
            .await?;
        Ok(response
            .tokens
            .into_iter()
            .map(|token| (token.mint, token))
            .collect())
    }

    fn page_path(
        path: &str,
        limit: u16,
        cursor: &crate::ws::types::Cursor,
    ) -> Result<String, HttpError> {
        let mut url = reqwest::Url::parse(&format!("http://localhost{path}"))
            .map_err(|_| HttpError::InvalidCursor)?;
        url.query_pairs_mut()
            .append_pair("limit", &limit.to_string());
        if let crate::ws::types::Cursor::After { id } = cursor {
            url.query_pairs_mut().append_pair("cursor", id);
        }
        Ok(match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        })
    }

    async fn collect<T: DeserializeOwned>(
        &self,
        path: &str,
        next: impl Fn(&T) -> Option<&str>,
        append: impl Fn(&mut T, T),
    ) -> Result<T, HttpError> {
        crate::paging::fetch_all(
            |cursor| async move { self.get(&Self::page_path(path, 200, &cursor)?).await },
            next,
            append,
        )
        .await
    }

    pub async fn venue(&self) -> Result<Venue, HttpError> {
        self.get("/api/v1/venue").await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpError> {
        let mut request = self.client.get(format!("{}{path}", self.base_url));
        if let Some(etag) = self.cached(path, |cached| cached.etag.clone()) {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let response = crate::io_observer::send(request, "acta", path).await?;
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED
            && let Some(body) = self.cached(path, |cached| Arc::clone(&cached.body))
        {
            return Ok(serde_json::from_slice(&body)?);
        }
        if !status.is_success() {
            let body =
                String::from_utf8_lossy(&crate::io_observer::bytes(response, "acta", path).await?)
                    .into_owned();
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
        let body = crate::io_observer::bytes(response, "acta", path).await?;
        let value = serde_json::from_slice(&body)?;
        if let Some(etag) = etag {
            let _ = self.cache().insert(
                path.to_owned(),
                Cached {
                    etag,
                    body: Arc::from(body.as_slice()),
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
