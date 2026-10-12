mod account;
mod crank;
mod error;
mod governance;
mod manager;
mod pda;
mod pyth;
mod served;
mod trading;

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
    CapitalBatchShape, VAULT_CAPITAL_BATCH_USERS, VaultAddresses, VaultManager, VaultManagerArgs,
    vault_capital_batch_compute_units,
};
pub use pyth::{PYTH_PUSH_ORACLE_ID, pyth_price_feed_account};
pub use served::{
    ComputeBudgetBounds, VaultIntentError, VaultOpenIntent, VaultSwapIntent,
    served_lookup_table_keys, verify_vault_open, verify_vault_swap,
};
pub use trading::{
    JUPITER_V6_PROGRAM_ID, MAX_VAULT_SWAP_ROUTE_ACCOUNTS, MAX_VAULT_SWAP_ROUTE_DATA,
    VaultOpenPosition, VaultPositionAddresses, VaultPositionRef, VaultSwapKind, VaultSwapRoute,
    VaultTradeTerms,
};

#[cfg(test)]
mod crank_tests;
#[cfg(test)]
mod golden_tests;
