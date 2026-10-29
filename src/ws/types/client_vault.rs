use crate::Address32;
use crate::types::ids::OrderId;
use crate::types::{VaultSwapDirection, VaultSwapMode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartVaultAuthData {
    pub operator: Address32,
    pub vault_pda: Address32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuoteData {
    pub request_id: Uuid,
    pub vault_pda: Address32,
    pub mode: VaultSwapMode,
    pub direction: VaultSwapDirection,
    pub amount: String,

    pub slippage_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapSubmitData {
    pub request_id: Uuid,
    pub execution_id: OrderId,
    pub signed_tx_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSwapStatusMessage {
    pub request_id: Uuid,
    pub execution_id: OrderId,
}
