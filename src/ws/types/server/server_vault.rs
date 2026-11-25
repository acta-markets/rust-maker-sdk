use crate::Address32;
use crate::types::ids::OrderId;
use crate::types::unix_time::UnixSeconds;
use crate::types::{VaultDelegatePermissions, VaultOperatorKind, VaultStagingStep, VaultSwapMode};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::time::SystemTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultAuthSuccessData {
    pub vault_pda: Address32,
    pub operator: Address32,
    pub operator_kind: VaultOperatorKind,
    pub permissions: VaultDelegatePermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultProjectionAdvancedData {
    pub vault_pda: Address32,
    pub version: u64,
    pub slot: u64,
    pub transaction_index: u32,
    pub signature: String,
    pub event_index: u32,
}

/// Sent to `Vaults` subscribers after a vault is created, re-delegated or closed; re-list vaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCatalogChangedData {
    pub vault_pda: Address32,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuote {
    /// Repeats the reply's `request_id` so a signer can bind the quote to its request offline.
    pub request_id: Uuid,
    pub execution_id: OrderId,
    pub mode: VaultSwapMode,
    pub tx_base64: String,
    pub last_valid_block_height: u64,
    pub in_amount: String,
    pub quoted_out_amount: String,
    pub minimum_out_amount: String,
    #[serde_as(as = "UnixSeconds")]
    pub valid_until: SystemTime,
    /// `none` for one-transaction swaps; once `stage` confirms, re-request (new id) for `unstage`.
    pub staging: SwapStaging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwapStaging {
    None,
    Stage,
    Unstage,
}

impl SwapStaging {
    #[must_use]
    pub const fn step(self) -> Option<VaultStagingStep> {
        match self {
            Self::None => None,
            Self::Stage => Some(VaultStagingStep::Stage),
            Self::Unstage => Some(VaultStagingStep::Unstage),
        }
    }
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
        amounts: SwapAmounts,
    },
    /// The keeper gave up; `reason` is its last error.
    Failed {
        reason: String,
        transaction: SwapTransaction,
    },
    /// Never landed before the quote's `valid_until`.
    Expired,
    /// No handoff of this execution for this Vault is known. Not proof of non-execution.
    Unknown,
}

/// What the swap moved, from its projected chain event.
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SwapAmounts {
    Projected {
        #[serde_as(as = "serde_with::DisplayFromStr")]
        amount_in: u64,
        #[serde_as(as = "serde_with::DisplayFromStr")]
        amount_out: u64,
    },
    /// Not projected yet; `HttpClient::vault_swaps` will carry it.
    NotProjected,
}

/// Whether the failed swap reached the chain; a sent one can be inspected by signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SwapTransaction {
    Sent { signature: String },
    NotSent,
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
    pub vault_pda: Address32,
    pub execution_id: OrderId,
    pub state: SwapExecutionState,
}
