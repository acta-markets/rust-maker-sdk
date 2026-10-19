use solana_sdk::pubkey::Pubkey;
use strum::EnumString;

use super::VaultChainError;
use crate::chain::ix::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};

/// Capital-cycle phase; parses the backend listing's snake_case `phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum VaultPhase {
    Idle,
    Active,
    Settling,
    ProcessingCapital,
}

impl TryFrom<u8> for VaultPhase {
    type Error = VaultChainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Idle),
            1 => Ok(Self::Active),
            2 => Ok(Self::Settling),
            3 => Ok(Self::ProcessingCapital),
            other => Err(VaultChainError::UnknownPhase(other)),
        }
    }
}

/// Safety mode; parses the backend listing's snake_case `safety`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum VaultSafety {
    Normal,
    Frozen,
    EmergencyUnwind,
    WithdrawOnly,
    Closing,
}

impl TryFrom<u8> for VaultSafety {
    type Error = VaultChainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Normal),
            1 => Ok(Self::Frozen),
            2 => Ok(Self::EmergencyUnwind),
            3 => Ok(Self::WithdrawOnly),
            4 => Ok(Self::Closing),
            other => Err(VaultChainError::UnknownSafety(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct VaultAccount {
    pub vault_id: [u8; 32],
    pub phase: VaultPhase,
    pub safety: VaultSafety,
    pub equity_computed: bool,
    pub accepting_deposits: bool,
    pub governance_authority: Pubkey,
    pub primary_delegate: Pubkey,
    pub main_mint: Pubkey,
    pub second_mint: Pubkey,
    pub share_mint: Pubkey,
    pub main_token_program: Pubkey,
    pub second_token_program: Pubkey,
    /// Positions the vault is party to that have not settled or been liquidated.
    pub open_positions: u64,
    pub total_shares: u64,
    pub pending_deposit_count: u32,
    pub pending_withdraw_count: u32,
    pub capital_fallback_delay_secs: u64,
    pub closing_started_at: u64,
    pub boundary_equity: u64,
    pub boundary_shares: u64,
}

const VAULT_DISCRIMINATOR: u8 = 9;
const VAULT_VERSION: u8 = 1;
const REQUEST_VERSION: u8 = 1;
const EQUITY_COMPUTED: u32 = 1 << 0;
const ACCEPTING_DEPOSITS: u32 = 1 << 1;

impl VaultAccount {
    pub const LEN: usize = 712;

    /// [`Self::parse`] for an account that must be owned by `program_id`.
    pub fn from_account(
        account: &solana_sdk::account::Account,
        program_id: &Pubkey,
    ) -> Result<Self, VaultChainError> {
        if account.owner != *program_id {
            return Err(VaultChainError::Owner {
                expected: *program_id,
                actual: account.owner,
            });
        }
        Self::parse(&account.data)
    }

    pub fn parse(data: &[u8]) -> Result<Self, VaultChainError> {
        if data.len() != Self::LEN {
            return Err(VaultChainError::AccountLength {
                expected: Self::LEN,
                actual: data.len(),
            });
        }
        let data = header(data, Self::LEN, VAULT_DISCRIMINATOR, VAULT_VERSION)?;
        let flags = u32_at(data, 12);
        if flags & !(EQUITY_COMPUTED | ACCEPTING_DEPOSITS) != 0 {
            return Err(VaultChainError::UnknownAccountingFlags(flags));
        }
        Ok(Self {
            vault_id: bytes32(data, 16),
            phase: VaultPhase::try_from(data[3])?,
            safety: VaultSafety::try_from(data[4])?,
            equity_computed: flags & EQUITY_COMPUTED != 0,
            accepting_deposits: flags & ACCEPTING_DEPOSITS != 0,
            governance_authority: pubkey(data, 48),
            primary_delegate: pubkey(data, 144),
            main_mint: pubkey(data, 176),
            second_mint: pubkey(data, 208),
            share_mint: pubkey(data, 240),
            main_token_program: token_program(data[402])?,
            second_token_program: token_program(data[403])?,
            open_positions: u64_at(data, 280),
            total_shares: u64_at(data, 288),
            pending_deposit_count: u32_at(data, 320),
            pending_withdraw_count: u32_at(data, 324),
            capital_fallback_delay_secs: u64_at(data, 408),
            closing_started_at: u64_at(data, 432),
            boundary_equity: u64_at(data, 440),
            boundary_shares: u64_at(data, 448),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultRequestKind {
    Deposit,
    Withdraw,
}

impl VaultRequestKind {
    #[must_use]
    pub const fn discriminator(self) -> u8 {
        match self {
            Self::Deposit => 12,
            Self::Withdraw => 13,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultRequest {
    pub user: Pubkey,
    pub vault: Pubkey,
    /// Main-mint atoms for a deposit, shares for a withdrawal.
    pub amount: u64,
    pub created_at: u64,
}

impl VaultRequest {
    pub const LEN: usize = 344;
    pub const VAULT_OFFSET: usize = 40;

    /// [`Self::parse`] for an account that must be owned by `program_id` and sized exactly.
    pub fn from_account(
        account: &solana_sdk::account::Account,
        program_id: &Pubkey,
        kind: VaultRequestKind,
    ) -> Result<Self, VaultChainError> {
        if account.owner != *program_id {
            return Err(VaultChainError::Owner {
                expected: *program_id,
                actual: account.owner,
            });
        }
        if account.data.len() != Self::LEN {
            return Err(VaultChainError::AccountLength {
                expected: Self::LEN,
                actual: account.data.len(),
            });
        }
        Self::parse(&account.data, kind)
    }

    pub fn parse(data: &[u8], kind: VaultRequestKind) -> Result<Self, VaultChainError> {
        let data = header(data, Self::LEN, kind.discriminator(), REQUEST_VERSION)?;
        Ok(Self {
            user: pubkey(data, 8),
            vault: pubkey(data, Self::VAULT_OFFSET),
            amount: u64_at(data, 72),
            created_at: u64_at(data, 80),
        })
    }
}

const ORACLE_SOURCE_LEN: usize = 424;
const ORACLE_SOURCE_FEEDS: usize = 40;
const PYTH_ORACLE_TYPE: usize = 1;

pub fn oracle_source_pyth_feed(data: &[u8]) -> Result<[u8; 32], VaultChainError> {
    let data = header(data, ORACLE_SOURCE_LEN, 7, 1)?;
    let feed = bytes32(data, ORACLE_SOURCE_FEEDS + PYTH_ORACLE_TYPE * 32);
    if feed == [0; 32] {
        return Err(VaultChainError::MissingPythFeed);
    }
    Ok(feed)
}

fn header(
    data: &[u8],
    len: usize,
    discriminator: u8,
    version: u8,
) -> Result<&[u8], VaultChainError> {
    let Some(data) = data.get(..len) else {
        return Err(VaultChainError::AccountTooShort {
            expected: len,
            actual: data.len(),
        });
    };
    if data[0] != discriminator {
        return Err(VaultChainError::Discriminator {
            expected: discriminator,
            actual: data[0],
        });
    }
    if data[1] != version {
        return Err(VaultChainError::Version(data[1]));
    }
    Ok(data)
}

fn token_program(kind: u8) -> Result<Pubkey, VaultChainError> {
    match kind {
        0 => Ok(TOKEN_PROGRAM_ID),
        1 => Ok(TOKEN_2022_PROGRAM_ID),
        other => Err(VaultChainError::UnknownTokenProgramKind(other)),
    }
}

fn bytes32(data: &[u8], offset: usize) -> [u8; 32] {
    let mut out = [0; 32];
    out.copy_from_slice(&data[offset..offset + 32]);
    out
}

fn pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(bytes32(data, offset))
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    let mut out = [0; 4];
    out.copy_from_slice(&data[offset..offset + 4]);
    u32::from_le_bytes(out)
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    let mut out = [0; 8];
    out.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(out)
}
