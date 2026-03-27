use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketInfo {
    pub pda: String,
    pub underlying: String,
    pub quote: String,
    #[serde_as(as = "TimestampSeconds<i64>")]
    pub expiry_ts: SystemTime,
    pub is_put: bool,
}
