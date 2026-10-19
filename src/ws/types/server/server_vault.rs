use crate::types::ids::OrderId;
use crate::types::{VaultDelegatePermissions, VaultOperatorKind, VaultSwapMode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultAuthSuccessData {
    pub vault_pda: String,
    pub operator: String,
    pub operator_kind: VaultOperatorKind,
    pub permissions: VaultDelegatePermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultProjectionAdvancedData {
    pub vault_pda: String,
    pub version: u64,
    pub slot: u64,
    pub transaction_index: u32,
    pub signature: String,
    pub event_index: u32,
}

/// Sent to `Vaults` subscribers after a vault is created, re-delegated or closed; re-list vaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCatalogChangedData {
    pub vault_pda: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuoteResultData {
    pub request_id: Uuid,
    pub execution_id: OrderId,
    pub mode: VaultSwapMode,
    pub tx_base64: String,
    pub last_valid_block_height: u64,
    pub in_amount: String,
    pub quoted_out_amount: String,
    pub minimum_out_amount: String,
    pub valid_until: String,
    pub main_price_update: String,
    pub second_price_update: String,
}

/// Durable enqueue acknowledgement, not chain confirmation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapSubmitResultData {
    pub request_id: Uuid,
    pub execution_id: OrderId,
}

/// Keeper knowledge about one Vault swap execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SwapExecutionState {
    /// Handed to the keeper; the outcome is unresolved.
    Pending,
    Finalized {
        signature: String,
        slot: u64,
    },
    Failed,
    Expired,
    /// No handoff of this execution for this Vault is known. Not proof of non-execution.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapStatusMessage {
    pub request_id: Uuid,
    pub execution_id: OrderId,
    pub state: SwapExecutionState,
}

/// Pushed to the Vault's sessions when a swap reaches a terminal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapSettledMessage {
    pub vault_pda: String,
    pub execution_id: OrderId,
    pub state: SwapExecutionState,
}
