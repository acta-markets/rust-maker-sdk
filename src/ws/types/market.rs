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
    pub underlying: Address32,
    pub quote: Address32,
    #[serde_as(as = "UnixSeconds")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
    pub status: MarketStatus,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub settlement_price: Option<Price>,
}
