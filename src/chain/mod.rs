mod accounts;
pub mod batch;
mod error;
pub mod ix;

#[cfg(feature = "chain-rpc")]
pub mod confirmation;
#[cfg(feature = "chain-rpc")]
pub mod rpc;

mod settlement;

pub mod vault;

pub mod wsol;

pub use accounts::{GlobalConfigInfo, MakerInfo, MarketInfo, PositionInfo, PositionStatus};
pub use batch::{
    MakerSnapshot, PositionSnapshot, PremiumBalance, PremiumToken, decode_premium_balance,
};
#[cfg(feature = "chain-rpc")]
pub use confirmation::ConfirmationStatus;
pub use error::ChainError;
pub use ix::{
    ChainIxError, FundPositionIxArgs, MakerDepositIxArgs, MakerWithdrawIxArgs,
    UpdateQuoteSigningIxArgs, build_fund_position_ixs, build_maker_deposit_ixs,
    build_maker_withdraw_ixs, build_update_quote_signing_ixs, derive_associated_token_address,
    maker_pda_with_program_id,
};

#[cfg(feature = "chain-rpc")]
pub use rpc::{ChainClient, FundPositionArgs, MakerDepositArgs, MakerWithdrawArgs, SendOptions};

pub use wsol::{
    DEFAULT_SOL_RESERVE_LAMPORTS, NATIVE_MINT, NativeSolFunding,
    build_maker_deposit_native_sol_ixs, build_maker_withdraw_native_sol_ixs, unwrap_account,
};

pub mod events;
pub mod spreads;
