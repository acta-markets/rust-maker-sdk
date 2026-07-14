use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

use super::ix::ChainIxError;

#[derive(Debug, Error)]
pub enum ChainError {
    #[cfg(feature = "chain-rpc")]
    #[error(transparent)]
    Rpc(#[from] solana_client::client_error::ClientError),
    #[error("invalid account data")]
    InvalidAccountData,
    #[error("account owner mismatch: expected {expected}, got {actual}")]
    AccountOwnerMismatch { expected: Pubkey, actual: Pubkey },
    #[error("account discriminator mismatch: expected {expected}, got {actual}")]
    AccountDiscriminatorMismatch { expected: u8, actual: u8 },
    #[error("account is not initialized")]
    UninitializedAccount,
    #[error("unknown position status: {0}")]
    UnknownPositionStatus(u8),
    #[error("invalid position type: {0}")]
    InvalidPositionType(u8),
    #[error("settlement amount overflow")]
    SettlementAmountOverflow,
    #[error("funding token account does not exist: {0}")]
    MissingFundingAccount(Pubkey),
    #[error("native SOL wrapping requires the classic SPL Token program, got {0}")]
    InvalidNativeTokenProgram(Pubkey),
    #[error(
        "insufficient native SOL to wrap: needed={needed_lamports} available={available_lamports}"
    )]
    InsufficientNativeSol {
        needed_lamports: u64,
        available_lamports: u64,
    },
    #[error("native SOL budget overflow")]
    NativeSolBudgetOverflow,
    #[error(transparent)]
    Ix(#[from] ChainIxError),
    #[cfg(feature = "chain-rpc")]
    #[error("transaction signing failed: {0}")]
    Signing(#[from] solana_sdk::signer::SignerError),
}
