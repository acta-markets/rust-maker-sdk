use crate::types::VaultSwapMode;
use crate::types::ids::OrderId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartVaultAuthData {
    pub operator: String,
    pub vault_pda: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuoteData {
    pub request_id: Uuid,
    pub vault_pda: String,
    pub mode: VaultSwapMode,
    pub input_mint: String,
    pub output_mint: String,
    pub amount: String,
    #[serde(default = "default_swap_slippage_bps")]
    pub slippage_bps: u16,
}

const fn default_swap_slippage_bps() -> u16 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapSubmitData {
    pub request_id: Uuid,
    pub execution_id: OrderId,
    pub signed_tx_base64: String,
}
