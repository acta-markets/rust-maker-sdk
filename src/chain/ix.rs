use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChainIxError {
    #[error("invalid position type: {0}")]
    InvalidPositionType(u8),
}

#[derive(Debug, Clone)]
pub struct MakerDepositIxArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Pubkey,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct MakerWithdrawIxArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Pubkey,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct FundPositionIxArgs {
    pub holder: Pubkey,
    pub position_pda: Pubkey,
    pub market_pda: Pubkey,
    pub position_type: u8,
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub underlying_token_program_id: Pubkey,
    pub quote_token_program_id: Pubkey,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateQuoteSigningIxArgs {
    pub maker_owner: Pubkey,
    pub new_quote_signing: Pubkey,
}

pub fn build_maker_deposit_ixs(
    program_id: &Pubkey,
    args: &MakerDepositIxArgs,
    fee_payer: Pubkey,
) -> Result<Vec<Instruction>, ChainIxError> {
    let (maker_pda, _) = maker_pda_with_program_id(program_id, &args.maker_owner);
    let maker_owner_premium_ata =
        derive_associated_token_address(&args.maker_owner, &args.premium_mint, &args.token_program);
    let maker_custody_ata =
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
            &maker_custody_ata,
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
            AccountMeta::new(maker_custody_ata, false),
            AccountMeta::new_readonly(args.token_program, false),
        ],
        data: build_amount_instruction(InstructionKind::MakerDeposit, args.amount),
    };

    ixs.push(ix);
    Ok(ixs)
}

pub fn build_maker_withdraw_ixs(
    program_id: &Pubkey,
    args: &MakerWithdrawIxArgs,
    fee_payer: Pubkey,
) -> Result<Vec<Instruction>, ChainIxError> {
    let (maker_pda, _) = maker_pda_with_program_id(program_id, &args.maker_owner);
    let maker_owner_premium_ata =
        derive_associated_token_address(&args.maker_owner, &args.premium_mint, &args.token_program);
    let maker_custody_ata =
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
            AccountMeta::new(maker_custody_ata, false),
            AccountMeta::new(maker_owner_premium_ata, false),
            AccountMeta::new_readonly(args.token_program, false),
        ],
        data: build_amount_instruction(InstructionKind::MakerWithdraw, args.amount),
    };

    ixs.push(ix);
    Ok(ixs)
}

/// Rotate the maker's quote-signing key; `maker_owner` signs.
pub fn build_update_quote_signing_ixs(
    program_id: &Pubkey,
    args: &UpdateQuoteSigningIxArgs,
) -> Vec<Instruction> {
    let (maker_pda, _) = maker_pda_with_program_id(program_id, &args.maker_owner);
    let mut data = Vec::with_capacity(1 + 32);
    data.push(InstructionKind::UpdateMakerQuoteSigning as u8);
    data.extend_from_slice(args.new_quote_signing.as_ref());
    vec![Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(args.maker_owner, true),
            AccountMeta::new(maker_pda, false),
        ],
        data,
    }]
}

pub fn build_fund_position_ixs(
    program_id: &Pubkey,
    args: &FundPositionIxArgs,
    fee_payer: Pubkey,
) -> Result<Vec<Instruction>, ChainIxError> {
    let funds = PositionFundsAccounts::new(program_id, args)?;
    let mut ixs = Vec::new();
    if args.create_atas {
        ixs.push(funds.create_holder_ata(&fee_payer));
        ixs.push(create_associated_token_account_idempotent_ix(
            &fee_payer,
            &funds.pos_settlement_ata,
            &args.position_pda,
            &funds.settlement_mint,
            &funds.settlement_program,
        ));
    }
    ixs.push(funds.instruction(InstructionKind::DepositFundsToPosition));
    Ok(ixs)
}

/// Return a Funded position's deposit to its holder; native SOL stays wrapped.
pub fn build_withdraw_position_funds_ixs(
    program_id: &Pubkey,
    args: &FundPositionIxArgs,
    fee_payer: Pubkey,
) -> Result<Vec<Instruction>, ChainIxError> {
    let funds = PositionFundsAccounts::new(program_id, args)?;
    let mut ixs = Vec::new();
    if args.create_atas {
        ixs.push(funds.create_holder_ata(&fee_payer));
    }
    ixs.push(funds.instruction(InstructionKind::WithdrawFundsFromPosition));
    Ok(ixs)
}

struct PositionFundsAccounts<'a> {
    program_id: &'a Pubkey,
    args: &'a FundPositionIxArgs,
    maker_pda: Pubkey,
    settlement_mint: Pubkey,
    settlement_program: Pubkey,
    holder_funding_ata: Pubkey,
    pos_settlement_ata: Pubkey,
}

impl<'a> PositionFundsAccounts<'a> {
    fn new(program_id: &'a Pubkey, args: &'a FundPositionIxArgs) -> Result<Self, ChainIxError> {
        let (settlement_mint, settlement_program) = match args.position_type {
            0 => (args.quote_mint, args.quote_token_program_id),
            1 => (args.underlying_mint, args.underlying_token_program_id),
            other => return Err(ChainIxError::InvalidPositionType(other)),
        };
        Ok(Self {
            program_id,
            args,
            maker_pda: maker_pda_with_program_id(program_id, &args.holder).0,
            settlement_mint,
            settlement_program,
            holder_funding_ata: derive_associated_token_address(
                &args.holder,
                &settlement_mint,
                &settlement_program,
            ),
            pos_settlement_ata: derive_associated_token_address(
                &args.position_pda,
                &settlement_mint,
                &settlement_program,
            ),
        })
    }

    fn create_holder_ata(&self, fee_payer: &Pubkey) -> Instruction {
        create_associated_token_account_idempotent_ix(
            fee_payer,
            &self.holder_funding_ata,
            &self.args.holder,
            &self.settlement_mint,
            &self.settlement_program,
        )
    }

    fn instruction(&self, kind: InstructionKind) -> Instruction {
        Instruction {
            program_id: *self.program_id,
            accounts: vec![
                AccountMeta::new(self.args.holder, true),
                AccountMeta::new_readonly(self.maker_pda, false),
                AccountMeta::new(self.args.position_pda, false),
                AccountMeta::new_readonly(self.args.market_pda, false),
                AccountMeta::new(self.holder_funding_ata, false),
                AccountMeta::new(self.pos_settlement_ata, false),
                AccountMeta::new_readonly(self.settlement_program, false),
            ],
            data: vec![kind as u8],
        }
    }
}

#[repr(u8)]
enum InstructionKind {
    UpdateMakerQuoteSigning = 3,
    MakerDeposit = 4,
    MakerWithdraw = 5,
    DepositFundsToPosition = 9,
    WithdrawFundsFromPosition = 81,
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

pub(crate) fn position_pda(program_id: &Pubkey, order_id: &[u8; 32]) -> Pubkey {
    Pubkey::find_program_address(&[b"position", order_id], program_id).0
}

pub fn derive_associated_token_address(
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    derive_associated_token_address_with_bump(owner, mint, token_program).0
}

pub(crate) fn derive_associated_token_address_with_bump(
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
}

pub(crate) fn create_associated_token_account_idempotent_ix(
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

pub(crate) const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub(crate) const SYSTEM_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("11111111111111111111111111111111");
const RENT_SYSVAR_ID: Pubkey =
    solana_pubkey::pubkey!("SysvarRent111111111111111111111111111111111");
pub(crate) const TOKEN_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub(crate) const TOKEN_2022_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
