use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::ChainError;

/// Default amount left in the maker wallet after wrapping, for fee headroom.
pub const DEFAULT_SOL_RESERVE_LAMPORTS: u64 = 1_200_000;
pub const NATIVE_MINT: Pubkey = solana_sdk::pubkey!("So11111111111111111111111111111111111111112");

const TOKEN_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const SYSTEM_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("11111111111111111111111111111111");
const SPL_TOKEN_SYNC_NATIVE: u8 = 17;
const SYSTEM_INSTRUCTION_TRANSFER: u32 = 2;
const TOKEN_ACCOUNT_MINT_OFFSET: usize = 0;
const TOKEN_ACCOUNT_OWNER_OFFSET: usize = 32;
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_BASE_LEN: usize = 72;
const TOKEN_ACCOUNT_RENT_LEN: usize = 165;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeSolFunding {
    #[default]
    Disabled,
    WrapIfNeeded {
        reserve_lamports: u64,
    },
}

impl NativeSolFunding {
    #[must_use]
    pub const fn with_default_reserve() -> Self {
        Self::WrapIfNeeded {
            reserve_lamports: DEFAULT_SOL_RESERVE_LAMPORTS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WrapDecision {
    NotNeeded,
    Wrap {
        wrap_lamports: u64,
    },
    InsufficientNativeSol {
        needed_lamports: u64,
        available_lamports: u64,
    },
}

pub(crate) enum WrapPlan {
    NotNeeded,
    Wrap {
        instructions: Vec<Instruction>,
        budget: NativeSolBudget,
    },
}

pub(crate) struct WsolWrapRequest {
    pub(crate) owner: Pubkey,
    pub(crate) funding_ata: Pubkey,
    pub(crate) token_program: Pubkey,
    pub(crate) expected_settlement: u64,
    pub(crate) reserve_lamports: u64,
    pub(crate) allow_account_creation: bool,
    pub(crate) fee_payer: Pubkey,
    pub(crate) additional_ata_rent_lamports: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeSolBudget {
    pub(crate) owner: Pubkey,
    wrap_lamports: u64,
    ata_rent_lamports: u64,
    reserve_lamports: u64,
}

impl NativeSolBudget {
    fn required_lamports(self, fee_lamports: u64) -> Result<u64, ChainError> {
        self.wrap_lamports
            .checked_add(self.ata_rent_lamports)
            .and_then(|value| value.checked_add(fee_lamports))
            .and_then(|value| value.checked_add(self.reserve_lamports))
            .ok_or(ChainError::NativeSolBudgetOverflow)
    }

    pub(crate) fn validate(
        self,
        available_lamports: u64,
        fee_lamports: u64,
    ) -> Result<(), ChainError> {
        let needed_lamports = self.required_lamports(fee_lamports)?;
        if available_lamports < needed_lamports {
            return Err(ChainError::InsufficientNativeSol {
                needed_lamports,
                available_lamports,
            });
        }
        Ok(())
    }
}

pub(crate) fn is_native_mint(mint: &Pubkey) -> bool {
    *mint == NATIVE_MINT
}

pub(crate) fn decide_wrap(
    existing_wsol: u64,
    expected_settlement: u64,
    sol_balance: u64,
    reserve_lamports: u64,
    ata_rent_lamports: u64,
) -> WrapDecision {
    if existing_wsol >= expected_settlement {
        return WrapDecision::NotNeeded;
    }
    let wrap_lamports = expected_settlement - existing_wsol;
    let needed_lamports = wrap_lamports
        .saturating_add(ata_rent_lamports)
        .saturating_add(reserve_lamports);
    if sol_balance < needed_lamports {
        return WrapDecision::InsufficientNativeSol {
            needed_lamports,
            available_lamports: sol_balance,
        };
    }
    WrapDecision::Wrap { wrap_lamports }
}

pub(crate) fn plan_wsol_wrap(
    rpc: &RpcClient,
    request: WsolWrapRequest,
) -> Result<WrapPlan, ChainError> {
    let WsolWrapRequest {
        owner,
        funding_ata,
        token_program,
        expected_settlement,
        reserve_lamports,
        allow_account_creation,
        fee_payer,
        additional_ata_rent_lamports,
    } = request;
    if token_program != TOKEN_PROGRAM_ID {
        return Err(ChainError::InvalidNativeTokenProgram(token_program));
    }

    let account = rpc
        .get_account_with_commitment(&funding_ata, rpc.commitment())
        .map_err(ChainError::from)?
        .value;
    let (existing_wsol, ata_rent_lamports) = match account {
        Some(account) => (parse_wsol_balance(&account, &owner, &token_program)?, 0),
        None if allow_account_creation => {
            let rent = if fee_payer == owner {
                rpc.get_minimum_balance_for_rent_exemption(TOKEN_ACCOUNT_RENT_LEN)?
            } else {
                0
            };
            (0, rent)
        }
        None => return Err(ChainError::MissingFundingAccount(funding_ata)),
    };
    if existing_wsol >= expected_settlement {
        return Ok(WrapPlan::NotNeeded);
    }
    let sol_balance = rpc.get_balance(&owner)?;

    match decide_wrap(
        existing_wsol,
        expected_settlement,
        sol_balance,
        reserve_lamports,
        ata_rent_lamports.saturating_add(additional_ata_rent_lamports),
    ) {
        WrapDecision::NotNeeded => Ok(WrapPlan::NotNeeded),
        WrapDecision::InsufficientNativeSol {
            needed_lamports,
            available_lamports,
        } => Err(ChainError::InsufficientNativeSol {
            needed_lamports,
            available_lamports,
        }),
        WrapDecision::Wrap { wrap_lamports } => Ok(WrapPlan::Wrap {
            instructions: vec![
                build_system_transfer_ix(&owner, &funding_ata, wrap_lamports),
                build_sync_native_ix(&token_program, &funding_ata),
            ],
            budget: NativeSolBudget {
                owner,
                wrap_lamports,
                ata_rent_lamports: ata_rent_lamports
                    .checked_add(additional_ata_rent_lamports)
                    .ok_or(ChainError::NativeSolBudgetOverflow)?,
                reserve_lamports,
            },
        }),
    }
}

pub(crate) fn token_account_rent_lamports(rpc: &RpcClient) -> Result<u64, ChainError> {
    rpc.get_minimum_balance_for_rent_exemption(TOKEN_ACCOUNT_RENT_LEN)
        .map_err(ChainError::from)
}

fn parse_wsol_balance(
    account: &Account,
    expected_owner: &Pubkey,
    token_program: &Pubkey,
) -> Result<u64, ChainError> {
    if account.owner != *token_program || account.data.len() < TOKEN_ACCOUNT_BASE_LEN {
        return Err(ChainError::InvalidAccountData);
    }
    if read_pubkey(&account.data, TOKEN_ACCOUNT_MINT_OFFSET)? != NATIVE_MINT
        || read_pubkey(&account.data, TOKEN_ACCOUNT_OWNER_OFFSET)? != *expected_owner
    {
        return Err(ChainError::InvalidAccountData);
    }
    read_u64(&account.data, TOKEN_ACCOUNT_AMOUNT_OFFSET)
}

fn read_pubkey(data: &[u8], offset: usize) -> Result<Pubkey, ChainError> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .ok_or(ChainError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ChainError::InvalidAccountData)?;
    Ok(Pubkey::new_from_array(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ChainError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(ChainError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ChainError::InvalidAccountData)?;
    Ok(u64::from_le_bytes(bytes))
}

fn build_system_transfer_ix(from: &Pubkey, to: &Pubkey, lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&SYSTEM_INSTRUCTION_TRANSFER.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM_PROGRAM_ID,
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, false)],
        data,
    }
}

fn build_sync_native_ix(token_program: &Pubkey, native_account: &Pubkey) -> Instruction {
    Instruction {
        program_id: *token_program,
        accounts: vec![AccountMeta::new(*native_account, false)],
        data: vec![SPL_TOKEN_SYNC_NATIVE],
    }
}

#[cfg(test)]
#[path = "wsol_tests.rs"]
mod tests;
