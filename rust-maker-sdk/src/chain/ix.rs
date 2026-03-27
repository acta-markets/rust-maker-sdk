use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChainIxError {
    #[error("invalid position type: {0}")]
    InvalidPositionType(u8),
}

#[derive(Debug, Clone)]
pub struct DepositPremiumIxArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Pubkey,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct WithdrawPremiumIxArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Pubkey,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct FundPositionIxArgs {
    pub maker_owner: Pubkey,
    pub position_pda: Pubkey,
    pub market_pda: Pubkey,
    pub position_type: u8,
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub underlying_token_program_id: Pubkey,
    pub quote_token_program_id: Pubkey,
    pub create_atas: bool,
}

pub fn build_deposit_premium_ixs(
    program_id: &Pubkey,
    args: &DepositPremiumIxArgs,
    fee_payer: Pubkey,
) -> Result<Vec<Instruction>, ChainIxError> {
    let (maker_pda, _) = maker_pda_with_program_id(program_id, &args.maker_owner);
    let maker_owner_premium_ata =
        derive_associated_token_address(&args.maker_owner, &args.premium_mint, &args.token_program);
    let maker_premium_ata =
        derive_associated_token_address(&maker_pda, &args.premium_mint, &args.token_program);

    let mut ixs = Vec::new();
    if args.create_atas {
        ixs.push(create_associated_token_account_idempotent_ix(
            &fee_payer,
            &maker_owner_premium_ata,
            &args.maker_owner,
            &args.premium_mint,
            &args.token_program,
        ));
        ixs.push(create_associated_token_account_idempotent_ix(
            &fee_payer,
            &maker_premium_ata,
            &maker_pda,
            &args.premium_mint,
            &args.token_program,
        ));
    }

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(args.maker_owner, true),
            AccountMeta::new_readonly(maker_pda, false),
            AccountMeta::new(maker_owner_premium_ata, false),
            AccountMeta::new(maker_premium_ata, false),
            AccountMeta::new_readonly(args.token_program, false),
        ],
        data: build_amount_instruction(InstructionKind::DepositPremium, args.amount),
    };

    ixs.push(ix);
    Ok(ixs)
}

pub fn build_withdraw_premium_ixs(
    program_id: &Pubkey,
    args: &WithdrawPremiumIxArgs,
    fee_payer: Pubkey,
) -> Result<Vec<Instruction>, ChainIxError> {
    let (maker_pda, _) = maker_pda_with_program_id(program_id, &args.maker_owner);
    let maker_owner_premium_ata =
        derive_associated_token_address(&args.maker_owner, &args.premium_mint, &args.token_program);
    let maker_premium_ata =
        derive_associated_token_address(&maker_pda, &args.premium_mint, &args.token_program);

    let mut ixs = Vec::new();
    if args.create_atas {
        ixs.push(create_associated_token_account_idempotent_ix(
            &fee_payer,
            &maker_owner_premium_ata,
            &args.maker_owner,
            &args.premium_mint,
            &args.token_program,
        ));
    }

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(args.maker_owner, true),
            AccountMeta::new(maker_pda, false),
            AccountMeta::new(maker_premium_ata, false),
            AccountMeta::new(maker_owner_premium_ata, false),
            AccountMeta::new_readonly(args.token_program, false),
        ],
        data: build_amount_instruction(InstructionKind::WithdrawPremium, args.amount),
    };

    ixs.push(ix);
    Ok(ixs)
}

pub fn build_fund_position_ixs(
    program_id: &Pubkey,
    args: &FundPositionIxArgs,
    fee_payer: Pubkey,
) -> Result<Vec<Instruction>, ChainIxError> {
    let (maker_pda, _) = maker_pda_with_program_id(program_id, &args.maker_owner);
    let (settlement_mint, settlement_program) = match args.position_type {
        0 => (args.quote_mint, args.quote_token_program_id),
        1 => (args.underlying_mint, args.underlying_token_program_id),
        other => return Err(ChainIxError::InvalidPositionType(other)),
    };

    let maker_funding_ata =
        derive_associated_token_address(&args.maker_owner, &settlement_mint, &settlement_program);
    let pos_settlement_ata =
        derive_associated_token_address(&args.position_pda, &settlement_mint, &settlement_program);

    let mut ixs = Vec::new();
    if args.create_atas {
        ixs.push(create_associated_token_account_idempotent_ix(
            &fee_payer,
            &maker_funding_ata,
            &args.maker_owner,
            &settlement_mint,
            &settlement_program,
        ));
        ixs.push(create_associated_token_account_idempotent_ix(
            &fee_payer,
            &pos_settlement_ata,
            &args.position_pda,
            &settlement_mint,
            &settlement_program,
        ));
    }

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(args.maker_owner, true),
            AccountMeta::new_readonly(maker_pda, false),
            AccountMeta::new(args.position_pda, false),
            AccountMeta::new_readonly(args.market_pda, false),
            AccountMeta::new(maker_funding_ata, false),
            AccountMeta::new(pos_settlement_ata, false),
            AccountMeta::new_readonly(settlement_program, false),
        ],
        data: vec![InstructionKind::DepositFundsToPosition as u8],
    };

    ixs.push(ix);
    Ok(ixs)
}

// On-chain program instruction discriminants (must match Solana program)
#[repr(u8)]
enum InstructionKind {
    DepositPremium = 4,
    WithdrawPremium = 5,
    DepositFundsToPosition = 9,
}

fn build_amount_instruction(kind: InstructionKind, amount: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(kind as u8);
    data.extend_from_slice(&amount.to_le_bytes());
    data
}

pub fn maker_pda_with_program_id(program_id: &Pubkey, maker_owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"maker", maker_owner.as_ref()], program_id)
}

pub fn derive_associated_token_address(
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

fn create_associated_token_account_idempotent_ix(
    payer: &Pubkey,
    ata: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*ata, false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(*token_program, false),
            AccountMeta::new_readonly(RENT_SYSVAR_ID, false),
        ],
        data: vec![1u8],
    }
}

const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("11111111111111111111111111111111");
const RENT_SYSVAR_ID: Pubkey = solana_sdk::pubkey!("SysvarRent111111111111111111111111111111111");
