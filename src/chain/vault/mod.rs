//! Vault capital operation on chain: PDAs, account decoders, instruction
//! builders, and the capital-cycle crank decision.

mod account;
mod crank;
mod error;
mod governance;
mod manager;
mod pda;
mod pyth;

pub use account::{
    VaultAccount, VaultPhase, VaultRequest, VaultRequestKind, VaultSafety, oracle_source_pyth_feed,
};
pub use crank::{
    CapitalStep, VaultView, WindowClock, WindowPolicy, WindowTracker, WithdrawalPlan, next_step,
    plan_withdrawals,
};
pub use error::VaultChainError;
pub use governance::{VaultGovernanceUpdate, VaultGovernanceUpdateKind, VaultPolicyUpdate};
pub use manager::{
    CAPITAL_TX_USERS, CapitalBatchShape, VAULT_CAPITAL_BATCH_USERS, VaultAddresses, VaultManager,
    VaultManagerArgs, vault_capital_batch_compute_units,
};
pub use pyth::{PYTH_PUSH_ORACLE_ID, pyth_price_feed_account};

#[cfg(test)]
mod crank_tests;
#[cfg(test)]
mod golden_tests;
