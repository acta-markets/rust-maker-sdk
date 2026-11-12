use crate::Address32;
use crate::types::ids::Price;
use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketStatus {
    Active,
    Closed,
    Finalized,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketInfo {
    pub products: crate::types::ProductSet,
    pub pda: Address32,
    pub underlying_mint: Address32,
    pub quote_mint: Address32,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub status: MarketStatus,
    pub settlement: Settlement,
}

/// The market's settlement fixing, once published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Settlement {
    Pending,
    Fixed { price: Price },
}
