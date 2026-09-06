use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use super::ids::{Balance, MarketId, Quantity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum RfqCloseReason {
    Expired,
    TakerCancelled,
    Filled,
    MarketExpired,
    LadderTimeout,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum QuoteFinalStatus {
    Expired,
    Outbid,
    Cancelled,
    Filled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum QuoteCancelReason {
    Requested,
    RiskCheck,
    RfqAccepted,
    /// The session enabled `cancel_on_disconnect` and disconnected.
    MakerDisconnected,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum RfqAvailableAgainReason {
    SignatureTimeout,
    TxFailed,
    TxBuildFailed,
    #[serde(other)]
    Unknown,
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
