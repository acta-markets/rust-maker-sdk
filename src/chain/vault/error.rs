use thiserror::Error;

use super::manager::VAULT_CAPITAL_BATCH_USERS;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum VaultChainError {
    #[error("vault has an open staging slot")]
    Staged,
    #[error("caller is not the current vault governance authority")]
    GovernanceAuthorityRequired,
    #[error("HWM reset cancellation requires protocol cold authority")]
    ColdAuthorityRequired,
    #[error("price section is required")]
    MissingPriceSection,
    #[error("account owner mismatch: expected {expected}, got {actual}")]
    Owner {
        expected: solana_sdk::pubkey::Pubkey,
        actual: solana_sdk::pubkey::Pubkey,
    },
    #[error("account data is {actual} bytes, expected at least {expected}")]
    AccountTooShort { expected: usize, actual: usize },
    #[error("account data is {actual} bytes, expected {expected}")]
    AccountLength { expected: usize, actual: usize },
    #[error("account discriminator mismatch: expected {expected}, got {actual}")]
    Discriminator { expected: u8, actual: u8 },
    #[error("unsupported account version {0}")]
    Version(u8),
    #[error("unknown vault phase {0}")]
    UnknownPhase(u8),
    #[error("unknown vault safety mode {0}")]
    UnknownSafety(u8),
    #[error("unknown token program kind {0}")]
    UnknownTokenProgramKind(u8),
    #[error("unknown vault accounting flags {0:#x}")]
    UnknownAccountingFlags(u32),
    #[error("unknown vault staged kind {0}")]
    UnknownStagedKind(u8),
    #[error("unknown vault staged leg {0}")]
    UnknownStagedLeg(u8),
    #[error("invalid price route: {0}")]
    PriceRoute(&'static str),
    #[error("vault second leg rolled since the manager was built; refresh it")]
    SecondLegRolled,
    #[error("vault batch exceeds {VAULT_CAPITAL_BATCH_USERS} users")]
    BatchTooLarge,
    #[error("invalid vault input: {0}")]
    InvalidInput(&'static str),
}
