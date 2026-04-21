use thiserror::Error;

use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    account::Account, instruction::Instruction, pubkey::Pubkey, signature::Signature,
    signer::Signer, transaction::Transaction,
};

use crate::chain::ix::{
    ChainIxError, DepositPremiumIxArgs, FundPositionIxArgs, WithdrawPremiumIxArgs,
    build_deposit_premium_ixs, build_fund_position_ixs, build_withdraw_premium_ixs,
};

#[derive(Debug, Error)]
pub enum ChainError {
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("invalid account data")]
    InvalidAccountData,
    #[error("unknown position status: {0}")]
    UnknownPositionStatus(u8),
    #[error(transparent)]
    Ix(#[from] ChainIxError),
}

pub struct ChainClient {
    rpc: RpcClient,
    program_id: Pubkey,
    commitment: CommitmentConfig,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SendOptions {
    pub compute_unit_limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DepositPremiumArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Option<Pubkey>,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct WithdrawPremiumArgs {
    pub maker_owner: Pubkey,
    pub amount: u64,
    pub premium_mint: Pubkey,
    pub token_program: Option<Pubkey>,
    pub create_atas: bool,
}

#[derive(Debug, Clone)]
pub struct FundPositionArgs {
    pub maker_owner: Pubkey,
    pub position_pda: Pubkey,
    pub create_atas: bool,
}

/// On-chain position status discriminant.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionStatus {
    None = 0,
    Open = 1,
    Funded = 2,
    Liquidated = 3,
    Settled = 4,
}

impl TryFrom<u8> for PositionStatus {
    type Error = ChainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Open),
            2 => Ok(Self::Funded),
            3 => Ok(Self::Liquidated),
            4 => Ok(Self::Settled),
            other => Err(ChainError::UnknownPositionStatus(other)),
        }
    }
}

/// Parsed data from a position on-chain account.
#[derive(Debug, Clone)]
pub struct PositionInfo {
    pub position_type: u8,
    pub status: PositionStatus,
    pub taker_owner: Pubkey,
    pub maker_owner: Pubkey,
    pub market_pda: Pubkey,
    pub strike: u64,
    pub quantity: u64,
    pub order_id: [u8; 32],
}

/// Parsed data from a market on-chain account.
#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub underlying_token_program_id: Pubkey,
    pub quote_token_program_id: Pubkey,
    pub underlying_decimals: u8,
    pub quote_decimals: u8,
}

impl ChainClient {
    pub fn program_id(&self) -> Pubkey {
        self.program_id
    }

    pub fn new(rpc_url: &str, program_id: Pubkey, commitment: CommitmentConfig) -> Self {
        let rpc = RpcClient::new_with_commitment(rpc_url.to_string(), commitment);
        Self {
            rpc,
            program_id,
            commitment,
        }
    }

    pub fn deposit_premium(
        &self,
        args: DepositPremiumArgs,
        maker_owner: &dyn Signer,
        fee_payer: Option<&dyn Signer>,
    ) -> Result<Signature, ChainError> {
        let payer = fee_payer.unwrap_or(maker_owner);
        let ixs = self.build_deposit_premium_ixs(&args, payer.pubkey())?;
        self.send_instructions_with_options(ixs, payer, &[maker_owner], SendOptions::default())
    }

    pub fn withdraw_premium(
        &self,
        args: WithdrawPremiumArgs,
        maker_owner: &dyn Signer,
        fee_payer: Option<&dyn Signer>,
    ) -> Result<Signature, ChainError> {
        let payer = fee_payer.unwrap_or(maker_owner);
        let ixs = self.build_withdraw_premium_ixs(&args, payer.pubkey())?;
        self.send_instructions_with_options(ixs, payer, &[maker_owner], SendOptions::default())
    }

    pub fn fund_position(
        &self,
        args: FundPositionArgs,
        maker_owner: &dyn Signer,
        fee_payer: Option<&dyn Signer>,
    ) -> Result<Signature, ChainError> {
        let payer = fee_payer.unwrap_or(maker_owner);
        let ixs = self.build_fund_position_ixs(&args, payer.pubkey())?;
        self.send_instructions_with_options(ixs, payer, &[maker_owner], SendOptions::default())
    }

    pub fn build_deposit_premium_ixs(
        &self,
        args: &DepositPremiumArgs,
        fee_payer: Pubkey,
    ) -> Result<Vec<Instruction>, ChainError> {
        let token_program = self.resolve_token_program(&args.premium_mint, args.token_program)?;
        build_deposit_premium_ixs(
            &self.program_id,
            &DepositPremiumIxArgs {
                maker_owner: args.maker_owner,
                amount: args.amount,
                premium_mint: args.premium_mint,
                token_program,
                create_atas: args.create_atas,
            },
            fee_payer,
        )
        .map_err(ChainError::from)
    }

    pub fn build_withdraw_premium_ixs(
        &self,
        args: &WithdrawPremiumArgs,
        fee_payer: Pubkey,
    ) -> Result<Vec<Instruction>, ChainError> {
        let token_program = self.resolve_token_program(&args.premium_mint, args.token_program)?;
        build_withdraw_premium_ixs(
            &self.program_id,
            &WithdrawPremiumIxArgs {
                maker_owner: args.maker_owner,
                amount: args.amount,
                premium_mint: args.premium_mint,
                token_program,
                create_atas: args.create_atas,
            },
            fee_payer,
        )
        .map_err(ChainError::from)
    }

    pub fn build_fund_position_ixs(
        &self,
        args: &FundPositionArgs,
        fee_payer: Pubkey,
    ) -> Result<Vec<Instruction>, ChainError> {
        let position_data = self.fetch_account(&args.position_pda)?;
        let pos = parse_position_basic(&position_data)?;
        let market_data = self.fetch_account(&pos.market_pda)?;
        let market = parse_market(&market_data)?;

        build_fund_position_ixs(
            &self.program_id,
            &FundPositionIxArgs {
                maker_owner: args.maker_owner,
                position_pda: args.position_pda,
                market_pda: pos.market_pda,
                position_type: pos.position_type,
                underlying_mint: market.underlying_mint,
                quote_mint: market.quote_mint,
                underlying_token_program_id: market.underlying_token_program_id,
                quote_token_program_id: market.quote_token_program_id,
                create_atas: args.create_atas,
            },
            fee_payer,
        )
        .map_err(ChainError::from)
    }

    /// Fetch and parse a position account. Returns full position details for pre-flight validation.
    pub fn fetch_position_info(&self, position_pda: &Pubkey) -> Result<PositionInfo, ChainError> {
        let data = self.fetch_account(position_pda)?;
        parse_position(&data)
    }

    pub fn fetch_market_info(&self, market_pda: &Pubkey) -> Result<MarketInfo, ChainError> {
        let market_data = self.fetch_account(market_pda)?;
        parse_market(&market_data)
    }

    pub fn fetch_market_quote_mint(&self, market_pda: &Pubkey) -> Result<Pubkey, ChainError> {
        Ok(self.fetch_market_info(market_pda)?.quote_mint)
    }

    /// Fetch the raw token balance (u64 atoms) of a token account.
    pub fn fetch_token_balance(&self, token_account: &Pubkey) -> Result<u64, ChainError> {
        let result = self
            .rpc
            .get_token_account_balance(token_account)
            .map_err(|err| ChainError::Rpc(err.to_string()))?;
        result
            .amount
            .parse::<u64>()
            .map_err(|_| ChainError::InvalidAccountData)
    }

    fn resolve_token_program(
        &self,
        mint: &Pubkey,
        token_program: Option<Pubkey>,
    ) -> Result<Pubkey, ChainError> {
        if let Some(program) = token_program {
            return ensure_supported_token_program(program);
        }
        let account = self
            .rpc
            .get_account(mint)
            .map_err(|err| ChainError::Rpc(err.to_string()))?;
        ensure_supported_token_program(account.owner)
    }

    fn fetch_account(&self, pubkey: &Pubkey) -> Result<Account, ChainError> {
        self.rpc
            .get_account(pubkey)
            .map_err(|err| ChainError::Rpc(err.to_string()))
    }

    pub fn send_instructions_with_options(
        &self,
        mut instructions: Vec<Instruction>,
        fee_payer: &dyn Signer,
        extra_signers: &[&dyn Signer],
        options: SendOptions,
    ) -> Result<Signature, ChainError> {
        if let Some(limit) = options.compute_unit_limit {
            instructions.insert(
                0,
                solana_compute_budget_interface::ComputeBudgetInstruction::set_compute_unit_limit(
                    limit,
                ),
            );
        }
        let recent = self
            .rpc
            .get_latest_blockhash()
            .map_err(|err| ChainError::Rpc(err.to_string()))?;

        let mut signers: Vec<&dyn Signer> = Vec::with_capacity(1 + extra_signers.len());
        signers.push(fee_payer);
        for signer in extra_signers {
            if signer.pubkey() != fee_payer.pubkey() {
                signers.push(*signer);
            }
        }

        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&fee_payer.pubkey()),
            &signers,
            recent,
        );
        self.rpc
            .send_and_confirm_transaction_with_spinner_and_commitment(&tx, self.commitment)
            .map_err(|err| ChainError::Rpc(err.to_string()))
    }
}

// ─── Position parsing ──────────────────────────────────────────────────────────
//
// On-chain layout:
//   discriminator    u8   @ 0
//   version          u8   @ 1
//   bump             u8   @ 2
//   position_type    u8   @ 3
//   status           u8   @ 4
//   flags            u8   @ 5
//   flags2           u8   @ 6
//   flags3           u8   @ 7
//   taker_owner   Pubkey  @ 8   (32 bytes)
//   maker_owner   Pubkey  @ 40  (32 bytes)
//   market        Pubkey  @ 72  (32 bytes)
//   strike           u64  @ 104 (8 bytes, LE)
//   quantity         u64  @ 112 (8 bytes, LE)
//   total_premium    u64  @ 120 (8 bytes)
//   order_id      [u8;32] @ 128 (32 bytes)

const POSITION_OFFSET_POSITION_TYPE: usize = 3;
const POSITION_OFFSET_STATUS: usize = 4;
const POSITION_OFFSET_TAKER_OWNER: usize = 8;
const POSITION_OFFSET_MAKER_OWNER: usize = 40;
const POSITION_OFFSET_MARKET: usize = 72;
const POSITION_OFFSET_STRIKE: usize = 104;
const POSITION_OFFSET_QUANTITY: usize = 112;
const POSITION_OFFSET_ORDER_ID: usize = 128;
const POSITION_MIN_LEN: usize = POSITION_OFFSET_ORDER_ID + 32; // 160

struct PositionBasic {
    position_type: u8,
    market_pda: Pubkey,
}

fn parse_position_basic(data: &Account) -> Result<PositionBasic, ChainError> {
    if data.data.len() < POSITION_MIN_LEN {
        return Err(ChainError::InvalidAccountData);
    }
    Ok(PositionBasic {
        position_type: data.data[POSITION_OFFSET_POSITION_TYPE],
        market_pda: read_pubkey(&data.data, POSITION_OFFSET_MARKET)?,
    })
}

fn parse_position(data: &Account) -> Result<PositionInfo, ChainError> {
    if data.data.len() < POSITION_MIN_LEN {
        return Err(ChainError::InvalidAccountData);
    }
    let status = PositionStatus::try_from(data.data[POSITION_OFFSET_STATUS])?;
    let mut order_id = [0u8; 32];
    order_id.copy_from_slice(&data.data[POSITION_OFFSET_ORDER_ID..POSITION_OFFSET_ORDER_ID + 32]);
    Ok(PositionInfo {
        position_type: data.data[POSITION_OFFSET_POSITION_TYPE],
        status,
        taker_owner: read_pubkey(&data.data, POSITION_OFFSET_TAKER_OWNER)?,
        maker_owner: read_pubkey(&data.data, POSITION_OFFSET_MAKER_OWNER)?,
        market_pda: read_pubkey(&data.data, POSITION_OFFSET_MARKET)?,
        strike: read_u64(&data.data, POSITION_OFFSET_STRIKE)?,
        quantity: read_u64(&data.data, POSITION_OFFSET_QUANTITY)?,
        order_id,
    })
}

// ─── Market parsing ────────────────────────────────────────────────────────────
//
// On-chain layout:
//   discriminator               u8   @ 0
//   version                     u8   @ 1
//   bump                        u8   @ 2
//   underlying_decimals         u8   @ 3
//   quote_decimals              u8   @ 4
//   flags                       u8   @ 5
//   reserved_byte1              u8   @ 6
//   reserved_byte2              u8   @ 7
//   expiry_ts                  u64   @ 8
//   settlement_price           u64   @ 16
//   open_positions_count       u64   @ 24
//   underlying_mint         Pubkey   @ 32  (32 bytes)
//   quote_mint              Pubkey   @ 64  (32 bytes)
//   underlying_token_program Pubkey  @ 96  (32 bytes)
//   quote_token_program     Pubkey   @ 128 (32 bytes)
//   underlying_oracle       Pubkey   @ 160 (32 bytes)
//   quote_oracle            Pubkey   @ 192 (32 bytes)
//
// NOTE: prior version had WRONG offsets (160/192) reading oracle addresses as token programs.

const MARKET_OFFSET_UNDERLYING_DECIMALS: usize = 3;
const MARKET_OFFSET_QUOTE_DECIMALS: usize = 4;
const MARKET_OFFSET_UNDERLYING_MINT: usize = 32;
const MARKET_OFFSET_QUOTE_MINT: usize = 64;
const MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM: usize = 96;
const MARKET_OFFSET_QUOTE_TOKEN_PROGRAM: usize = 128;
const MARKET_MIN_LEN: usize = MARKET_OFFSET_QUOTE_TOKEN_PROGRAM + 32; // 160

fn parse_market(data: &Account) -> Result<MarketInfo, ChainError> {
    if data.data.len() < MARKET_MIN_LEN {
        return Err(ChainError::InvalidAccountData);
    }
    Ok(MarketInfo {
        underlying_decimals: data.data[MARKET_OFFSET_UNDERLYING_DECIMALS],
        quote_decimals: data.data[MARKET_OFFSET_QUOTE_DECIMALS],
        underlying_mint: read_pubkey(&data.data, MARKET_OFFSET_UNDERLYING_MINT)?,
        quote_mint: read_pubkey(&data.data, MARKET_OFFSET_QUOTE_MINT)?,
        underlying_token_program_id: read_pubkey(
            &data.data,
            MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM,
        )?,
        quote_token_program_id: read_pubkey(&data.data, MARKET_OFFSET_QUOTE_TOKEN_PROGRAM)?,
    })
}

// ─── Byte-level helpers ────────────────────────────────────────────────────────

fn read_pubkey(data: &[u8], offset: usize) -> Result<Pubkey, ChainError> {
    if data.len() < offset + 32 {
        return Err(ChainError::InvalidAccountData);
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&data[offset..offset + 32]);
    Ok(Pubkey::new_from_array(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ChainError> {
    if data.len() < offset + 8 {
        return Err(ChainError::InvalidAccountData);
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[offset..offset + 8]);
    Ok(u64::from_le_bytes(bytes))
}

const TOKEN_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const TOKEN_2022_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

fn ensure_supported_token_program(program: Pubkey) -> Result<Pubkey, ChainError> {
    if program == TOKEN_PROGRAM_ID || program == TOKEN_2022_PROGRAM_ID {
        Ok(program)
    } else {
        Err(ChainError::InvalidAccountData)
    }
}
