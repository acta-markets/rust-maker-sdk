mod account;
#[cfg(feature = "chain-rpc")]
mod counterparty;
mod crank;
mod error;
mod governance;
mod manager;
mod pda;
mod pyth;
#[cfg(feature = "chain-rpc")]
mod scan;
mod served;
mod trading;

pub use account::{
    VaultAccount, VaultPhase, VaultRequest, VaultRequestKind, VaultSafety, oracle_source_pyth_feed,
};
#[cfg(feature = "chain-rpc")]
pub use counterparty::Counterparty;
pub use crank::{
    CapitalStep, VaultView, WindowClock, WindowPolicy, WindowTracker, WithdrawalPlan, next_step,
    plan_withdrawals,
};
pub use error::VaultChainError;
pub use governance::{VaultGovernanceUpdate, VaultGovernanceUpdateKind, VaultPolicyUpdate};
pub use manager::{
    CapitalBatchShape, DepositPayout, VAULT_CAPITAL_BATCH_USERS, VaultAddresses, VaultManager,
    VaultManagerArgs, WithdrawalPayout, vault_capital_batch_compute_units,
};
pub use pyth::{PYTH_PUSH_ORACLE_ID, pyth_price_feed_account};
#[cfg(feature = "chain-rpc")]
pub use scan::PendingCapitalRequests;
pub use served::{
    ComputeBudgetBounds, PairPriceFeeds, VaultIntentError, VaultOpenIntent, VaultSwapIntent,
    VerifiedTransaction, served_lookup_table_keys, verify_vault_open, verify_vault_swap,
};
pub use trading::{
    DEVNET_SWAP_ROUTER_ID, JUPITER_V6_PROGRAM_ID, MAX_VAULT_SWAP_ROUTE_ACCOUNTS,
    MAX_VAULT_SWAP_ROUTE_DATA, VaultOpenPosition, VaultPositionAddresses, VaultPositionRef,
    VaultSwapKind, VaultSwapRoute, VaultTradeTerms,
};

#[cfg(test)]
mod crank_tests;
#[cfg(test)]
mod golden_tests;
