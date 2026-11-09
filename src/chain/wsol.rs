#[cfg(feature = "chain-rpc")]
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
#[cfg(feature = "chain-rpc")]
use solana_sdk::account::Account;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

#[cfg(feature = "chain-rpc")]
use super::ChainError;

/// Default amount left in the maker wallet after wrapping, for fee headroom.
pub const DEFAULT_SOL_RESERVE_LAMPORTS: u64 = 1_200_000;
pub const NATIVE_MINT: Pubkey = solana_sdk::pubkey!("So11111111111111111111111111111111111111112");

pub(crate) const TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const SYSTEM_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("11111111111111111111111111111111");
const SPL_TOKEN_SYNC_NATIVE: u8 = 17;
const SPL_TOKEN_CLOSE_ACCOUNT: u8 = 9;
const SYSTEM_INSTRUCTION_TRANSFER: u32 = 2;
const SYSTEM_INSTRUCTION_CREATE_ACCOUNT_WITH_SEED: u32 = 3;
const SPL_TOKEN_TRANSFER: u8 = 3;
const SPL_TOKEN_INITIALIZE_ACCOUNT3: u8 = 18;
/// Rent-exempt minimum of a 165-byte token account.
const TOKEN_ACCOUNT_RENT_LAMPORTS: u64 = 2_039_280;
/// Seed of the owner's temporary wSOL account; it is created and closed in one transaction.
const UNWRAP_SEED: &str = "acta-unwrap";
#[cfg(feature = "chain-rpc")]
const TOKEN_ACCOUNT_MINT_OFFSET: usize = 0;
#[cfg(feature = "chain-rpc")]
const TOKEN_ACCOUNT_OWNER_OFFSET: usize = 32;
#[cfg(feature = "chain-rpc")]
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
#[cfg(feature = "chain-rpc")]
const TOKEN_ACCOUNT_BASE_LEN: usize = 72;
#[cfg(feature = "chain-rpc")]
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
#[cfg(feature = "chain-rpc")]
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

#[cfg(feature = "chain-rpc")]
pub(crate) enum WrapPlan {
    NotNeeded,
    Wrap {
        instructions: Vec<Instruction>,
        budget: NativeSolBudget,
    },
}

#[cfg(feature = "chain-rpc")]
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
#[cfg(feature = "chain-rpc")]
pub(crate) struct NativeSolBudget {
    pub(crate) owner: Pubkey,
    wrap_lamports: u64,
    ata_rent_lamports: u64,
    reserve_lamports: u64,
}

#[cfg(feature = "chain-rpc")]
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

#[cfg(feature = "chain-rpc")]
pub(crate) fn is_native_mint(mint: &Pubkey) -> bool {
    *mint == NATIVE_MINT
}

#[cfg(feature = "chain-rpc")]
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

#[cfg(feature = "chain-rpc")]
pub(crate) async fn plan_wsol_wrap(
    rpc: &RpcClient,
    account: Option<Account>,
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
    let (existing_wsol, ata_rent_lamports) = match account {
        Some(account) => (parse_wsol_balance(&account, &owner, &token_program)?, 0),
        None if allow_account_creation => {
            let rent = if fee_payer == owner {
                rpc.get_minimum_balance_for_rent_exemption(TOKEN_ACCOUNT_RENT_LEN)
                    .await?
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
    let sol_balance = rpc.get_balance(&owner).await?;

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

#[cfg(feature = "chain-rpc")]
pub(crate) async fn token_account_rent_lamports(rpc: &RpcClient) -> Result<u64, ChainError> {
    rpc.get_minimum_balance_for_rent_exemption(TOKEN_ACCOUNT_RENT_LEN)
        .await
        .map_err(ChainError::from)
}

#[cfg(feature = "chain-rpc")]
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

#[cfg(feature = "chain-rpc")]
fn read_pubkey(data: &[u8], offset: usize) -> Result<Pubkey, ChainError> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .ok_or(ChainError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ChainError::InvalidAccountData)?;
    Ok(Pubkey::new_from_array(bytes))
}

#[cfg(feature = "chain-rpc")]
fn read_u64(data: &[u8], offset: usize) -> Result<u64, ChainError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(ChainError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ChainError::InvalidAccountData)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Builds the explicit SOL-to-wSOL step; callers aggregate batch deficits and budget rent and fees.
pub fn build_wrap_sol_ixs(owner: Pubkey, fee_payer: Pubkey, lamports: u64) -> Vec<Instruction> {
    let ata = super::ix::derive_associated_token_address(&owner, &NATIVE_MINT, &TOKEN_PROGRAM_ID);
    vec![
        super::ix::create_associated_token_account_idempotent_ix(
            &fee_payer,
            &ata,
            &owner,
            &NATIVE_MINT,
            &TOKEN_PROGRAM_ID,
        ),
        build_system_transfer_ix(&owner, &ata, lamports),
        build_sync_native_ix(&TOKEN_PROGRAM_ID, &ata),
    ]
}

/// Wraps `lamports` of native SOL and deposits them into the Maker PDA custody; `maker_owner` signs.
pub fn build_maker_deposit_native_sol_ixs(
    program_id: &Pubkey,
    maker_owner: Pubkey,
    fee_payer: Pubkey,
    lamports: u64,
) -> Result<Vec<Instruction>, super::ChainIxError> {
    let args = super::MakerDepositIxArgs {
        maker_owner,
        amount: lamports,
        premium_mint: NATIVE_MINT,
        token_program: TOKEN_PROGRAM_ID,
        create_atas: true,
    };
    let mut ixs = super::build_maker_deposit_ixs(program_id, &args, fee_payer)?;
    let Some(deposit) = ixs.pop() else {
        return Ok(ixs);
    };
    let ata =
        super::ix::derive_associated_token_address(&maker_owner, &NATIVE_MINT, &TOKEN_PROGRAM_ID);
    ixs.push(build_system_transfer_ix(&maker_owner, &ata, lamports));
    ixs.push(build_sync_native_ix(&TOKEN_PROGRAM_ID, &ata));
    ixs.push(deposit);
    Ok(ixs)
}

/// Withdraws `lamports` of wSOL from the Maker PDA custody and unwraps exactly that amount: it
/// moves to a temporary wSOL account derived from the owner, which is closed in the same
/// transaction. Other wSOL in the owner's account stays wrapped. `maker_owner` signs and fronts
/// the temporary account's rent, which the close returns.
pub fn build_maker_withdraw_native_sol_ixs(
    program_id: &Pubkey,
    maker_owner: Pubkey,
    fee_payer: Pubkey,
    lamports: u64,
) -> Result<Vec<Instruction>, super::ChainIxError> {
    let args = super::MakerWithdrawIxArgs {
        maker_owner,
        amount: lamports,
        premium_mint: NATIVE_MINT,
        token_program: TOKEN_PROGRAM_ID,
        create_atas: true,
    };
    let mut ixs = super::build_maker_withdraw_ixs(program_id, &args, fee_payer)?;
    let ata =
        super::ix::derive_associated_token_address(&maker_owner, &NATIVE_MINT, &TOKEN_PROGRAM_ID);
    let temporary = unwrap_account(&maker_owner);
    ixs.extend([
        build_create_account_with_seed_ix(&maker_owner, &temporary),
        build_initialize_account3_ix(&temporary, &maker_owner),
        build_token_transfer_ix(&ata, &temporary, &maker_owner, lamports),
        build_close_account_ix(&TOKEN_PROGRAM_ID, &temporary, &maker_owner),
    ]);
    Ok(ixs)
}

/// The owner's temporary wSOL account used to unwrap an exact amount: `create_with_seed(owner,
/// "acta-unwrap", token program)`, hashed directly because the fixed seed cannot fail its checks.
pub fn unwrap_account(owner: &Pubkey) -> Pubkey {
    use sha2::{Digest, Sha256};
    let hash = Sha256::new()
        .chain_update(owner.as_ref())
        .chain_update(UNWRAP_SEED.as_bytes())
        .chain_update(TOKEN_PROGRAM_ID.as_ref())
        .finalize();
    Pubkey::new_from_array(hash.into())
}

fn build_create_account_with_seed_ix(owner: &Pubkey, account: &Pubkey) -> Instruction {
    let mut data = Vec::with_capacity(4 + 32 + 8 + UNWRAP_SEED.len() + 8 + 8 + 32);
    data.extend_from_slice(&SYSTEM_INSTRUCTION_CREATE_ACCOUNT_WITH_SEED.to_le_bytes());
    data.extend_from_slice(owner.as_ref());
    data.extend_from_slice(&(UNWRAP_SEED.len() as u64).to_le_bytes());
    data.extend_from_slice(UNWRAP_SEED.as_bytes());
    data.extend_from_slice(&TOKEN_ACCOUNT_RENT_LAMPORTS.to_le_bytes());
    data.extend_from_slice(&(TOKEN_ACCOUNT_RENT_LEN as u64).to_le_bytes());
    data.extend_from_slice(TOKEN_PROGRAM_ID.as_ref());
    Instruction {
        program_id: SYSTEM_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new(*account, false),
            AccountMeta::new_readonly(*owner, true),
        ],
        data,
    }
}

fn build_initialize_account3_ix(account: &Pubkey, owner: &Pubkey) -> Instruction {
    let mut data = Vec::with_capacity(33);
    data.push(SPL_TOKEN_INITIALIZE_ACCOUNT3);
    data.extend_from_slice(owner.as_ref());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*account, false),
            AccountMeta::new_readonly(NATIVE_MINT, false),
        ],
        data,
    }
}

fn build_token_transfer_ix(from: &Pubkey, to: &Pubkey, owner: &Pubkey, amount: u64) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(SPL_TOKEN_TRANSFER);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*from, false),
            AccountMeta::new(*to, false),
            AccountMeta::new_readonly(*owner, true),
        ],
        data,
    }
}

fn build_close_account_ix(token_program: &Pubkey, account: &Pubkey, owner: &Pubkey) -> Instruction {
    Instruction {
        program_id: *token_program,
        accounts: vec![
            AccountMeta::new(*account, false),
            AccountMeta::new(*owner, false),
            AccountMeta::new_readonly(*owner, true),
        ],
        data: vec![SPL_TOKEN_CLOSE_ACCOUNT],
    }
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

#[cfg(all(test, feature = "chain-rpc"))]
#[path = "wsol_tests.rs"]
mod tests;
