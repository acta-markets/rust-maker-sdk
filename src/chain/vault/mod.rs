mod account;
#[cfg(feature = "chain-rpc")]
mod counterparty;
mod crank;
#[cfg(all(feature = "chain-rpc", feature = "http"))]
pub mod curator;
mod error;
mod governance;
mod manager;
mod pda;
mod price_route;
pub mod router;
mod router_price;
#[cfg(feature = "chain-rpc")]
mod scan;
mod served;
mod trading;
pub mod venues;

pub use account::{
    VaultAccount, VaultPhase, VaultRequest, VaultRequestKind, VaultSafety, VaultStagedSlot,
    VaultStagingState,
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
pub use price_route::{
    AccountRatioBrick, FreshnessUnit, GrowthCapBrick, MAX_PRICE_BRICKS, MAX_PRICE_ROUTE_BYTES,
    MAX_RATIO_DISCRIMINATOR_LEN, MaturityDiscountBrick, MaturitySource, PYTH_PUSH_ORACLE_ID,
    PriceBrick, PriceRoute, PythBrick, RatioField, RatioFreshness, pyth_price_feed_account,
};
#[cfg(feature = "chain-rpc")]
pub use scan::PendingCapitalRequests;
pub use served::{
    ComputeBudgetBounds, PairPriceRoutes, VaultIntentError, VaultOpenIntent, VaultSwapIntent,
    VerifiedTransaction, served_lookup_table_keys, verify_vault_open, verify_vault_swap,
};
pub use trading::{
    MAX_VAULT_SWAP_HOPS, MAX_VAULT_SWAP_PATH_ACCOUNTS, MAX_VAULT_SWAP_ROUTE_ACCOUNTS,
    MAX_VAULT_SWAP_ROUTE_DATA, VaultOpenPosition, VaultPositionAddresses, VaultPositionRef,
    VaultStagingAsset, VaultSwapAmountOffset, VaultSwapHop, VaultSwapKind, VaultSwapPlan,
    VaultTradeTerms,
};

#[cfg(test)]
mod crank_tests;
#[cfg(test)]
mod golden_tests;

#[cfg(test)]
mod router_tests;
