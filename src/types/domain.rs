use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use super::ids::{Balance, MarketId, Quantity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuoteStatus {
    Pending,
    Best,
    Outbid,
    Filled,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RfqCloseReason {
    Expired,
    TakerCancelled,
    Filled,
    MarketExpired,
    LadderTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum QuoteFinalStatus {
    Expired,
    Outbid,
    Cancelled,
    Filled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum QuoteCancelReason {
    Requested,
    RiskCheck,
    RfqAccepted,
}

/// `Pending..Expired` mirror persisted order states; `NotFound` is wire-only.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OrderStatus {
    Pending,
    Submitted,
    Confirmed,
    Failed,
    Expired,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RfqAvailableAgainReason {
    SignatureTimeout,
    TxFailed,
    TxBuildFailed,
}

impl std::fmt::Display for RfqAvailableAgainReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCapInfo {
    pub underlying_mint: String,
    pub symbol: String,
    pub current_oi: Quantity,
    pub max_oi: Quantity,
    pub utilization: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCapInfo {
    pub market_id: MarketId,
    pub current_oi: Quantity,
    pub max_oi: Quantity,
    pub utilization: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerPositionCapInfo {
    pub current: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerNotionalCapInfo {
    pub underlying_mint: String,
    pub symbol: String,
    pub current: Quantity,
    pub limit: Quantity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerBalanceCapInfo {
    pub mint: String,
    pub symbol: String,
    pub decimals: u8,
    pub deposited: Balance,
    pub committed: Balance,
    pub available: Balance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteCapInfo {
    pub quote_mint: String,
    pub symbol: String,
    pub current_notional: Balance,
    pub max_notional: Balance,
    pub utilization: f64,
}
