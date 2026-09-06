#[cfg(feature = "chain-rpc")]
mod accounts;
mod error;
pub mod ix;

#[cfg(feature = "chain-rpc")]
pub mod rpc;

#[cfg(feature = "chain-rpc")]
mod settlement;

#[cfg(feature = "chain-rpc")]
pub mod wsol;

#[cfg(feature = "chain-rpc")]
pub use accounts::{MarketInfo, PositionInfo, PositionStatus};
pub use error::ChainError;
pub use ix::{
    ChainIxError, DepositPremiumIxArgs, FundPositionIxArgs, WithdrawPremiumIxArgs,
    build_deposit_premium_ixs, build_fund_position_ixs, build_withdraw_premium_ixs,
    derive_associated_token_address, maker_pda_with_program_id,
};

#[cfg(feature = "chain-rpc")]
pub use rpc::{
    ChainClient, DepositPremiumArgs, FundPositionArgs, SendOptions, WithdrawPremiumArgs,
};

#[cfg(feature = "chain-rpc")]
pub use wsol::{DEFAULT_SOL_RESERVE_LAMPORTS, NATIVE_MINT, NativeSolFunding};
