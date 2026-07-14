use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    account::Account, instruction::Instruction, pubkey::Pubkey, signature::Signature,
    signer::Signer, transaction::Transaction,
};

use crate::chain::ChainError;
use crate::chain::accounts::{MarketInfo, PositionInfo, parse_market, parse_position};
use crate::chain::ix::{
    DepositPremiumIxArgs, FundPositionIxArgs, WithdrawPremiumIxArgs, build_deposit_premium_ixs,
    build_fund_position_ixs, build_withdraw_premium_ixs,
};
use crate::chain::settlement::required_settlement;
use crate::chain::wsol::{
    NativeSolBudget, NativeSolFunding, WrapPlan, WsolWrapRequest, is_native_mint, plan_wsol_wrap,
    token_account_rent_lamports,
};

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
    pub native_sol: NativeSolFunding,
}

struct PreparedFundPosition {
    instructions: Vec<Instruction>,
    native_budget: Option<NativeSolBudget>,
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
        let prepared = self.prepare_fund_position(&args, payer.pubkey())?;
        self.send_instructions(
            prepared.instructions,
            payer,
            &[maker_owner],
            SendOptions::default(),
            prepared.native_budget,
        )
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
        Ok(self.prepare_fund_position(args, fee_payer)?.instructions)
    }

    fn prepare_fund_position(
        &self,
        args: &FundPositionArgs,
        fee_payer: Pubkey,
    ) -> Result<PreparedFundPosition, ChainError> {
        let position_data = self.fetch_account(&args.position_pda)?;
        let pos = parse_position(&position_data, &self.program_id)?;
        let market_data = self.fetch_account(&pos.market_pda)?;
        let market = parse_market(&market_data, &self.program_id)?;

        let mut fund_ixs = build_fund_position_ixs(
            &self.program_id,
            &FundPositionIxArgs {
                maker_owner: args.maker_owner,
                position_pda: args.position_pda,
                market_pda: pos.market_pda,
                position_type: pos.position_type.into(),
                underlying_mint: market.underlying_mint,
                quote_mint: market.quote_mint,
                underlying_token_program_id: market.underlying_token_program_id,
                quote_token_program_id: market.quote_token_program_id,
                create_atas: args.create_atas,
            },
            fee_payer,
        )
        .map_err(ChainError::from)?;

        let NativeSolFunding::WrapIfNeeded { reserve_lamports } = args.native_sol else {
            return Ok(PreparedFundPosition {
                instructions: fund_ixs,
                native_budget: None,
            });
        };
        let (settlement_mint, settlement_program) = match pos.position_type {
            crate::types::PositionType::CoveredCall => {
                (market.quote_mint, market.quote_token_program_id)
            }
            crate::types::PositionType::CashSecuredPut => {
                (market.underlying_mint, market.underlying_token_program_id)
            }
        };
        if !is_native_mint(&settlement_mint) {
            return Ok(PreparedFundPosition {
                instructions: fund_ixs,
                native_budget: None,
            });
        }
        let expected_settlement = required_settlement(
            pos.position_type.into(),
            pos.strike.value(),
            pos.quantity.value(),
            market.quote_decimals.value(),
            market.underlying_decimals.value(),
        )?;
        if expected_settlement == 0 {
            return Err(ChainError::InvalidAccountData);
        }
        let funding_ata = crate::chain::ix::derive_associated_token_address(
            &args.maker_owner,
            &settlement_mint,
            &settlement_program,
        );
        let position_funding_ata = crate::chain::ix::derive_associated_token_address(
            &args.position_pda,
            &settlement_mint,
            &settlement_program,
        );
        let additional_ata_rent_lamports = if args.create_atas
            && fee_payer == args.maker_owner
            && self
                .rpc
                .get_account_with_commitment(&position_funding_ata, self.commitment)?
                .value
                .is_none()
        {
            token_account_rent_lamports(&self.rpc)?
        } else {
            0
        };
        match plan_wsol_wrap(
            &self.rpc,
            WsolWrapRequest {
                owner: args.maker_owner,
                funding_ata,
                token_program: settlement_program,
                expected_settlement,
                reserve_lamports,
                allow_account_creation: args.create_atas,
                fee_payer,
                additional_ata_rent_lamports,
            },
        )? {
            WrapPlan::NotNeeded => Ok(PreparedFundPosition {
                instructions: fund_ixs,
                native_budget: None,
            }),
            WrapPlan::Wrap {
                instructions: wrap_ixs,
                budget,
            } => {
                let deposit_ix = fund_ixs.pop().ok_or(ChainError::InvalidAccountData)?;
                fund_ixs.extend(wrap_ixs);
                fund_ixs.push(deposit_ix);
                Ok(PreparedFundPosition {
                    instructions: fund_ixs,
                    native_budget: Some(budget),
                })
            }
        }
    }

    /// Fetch and parse a position account. Returns full position details for pre-flight validation.
    pub fn fetch_position_info(&self, position_pda: &Pubkey) -> Result<PositionInfo, ChainError> {
        let data = self.fetch_account(position_pda)?;
        parse_position(&data, &self.program_id)
    }

    pub fn fetch_market_info(&self, market_pda: &Pubkey) -> Result<MarketInfo, ChainError> {
        let market_data = self.fetch_account(market_pda)?;
        parse_market(&market_data, &self.program_id)
    }

    pub fn fetch_market_quote_mint(&self, market_pda: &Pubkey) -> Result<Pubkey, ChainError> {
        Ok(self.fetch_market_info(market_pda)?.quote_mint)
    }

    /// Fetch the raw token balance (u64 atoms) of a token account.
    pub fn fetch_token_balance(&self, token_account: &Pubkey) -> Result<u64, ChainError> {
        let result = self.rpc.get_token_account_balance(token_account)?;
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
        let account = self.rpc.get_account(mint)?;
        ensure_supported_token_program(account.owner)
    }

    fn fetch_account(&self, pubkey: &Pubkey) -> Result<Account, ChainError> {
        self.rpc.get_account(pubkey).map_err(ChainError::from)
    }

    pub fn send_instructions_with_options(
        &self,
        instructions: Vec<Instruction>,
        fee_payer: &dyn Signer,
        extra_signers: &[&dyn Signer],
        options: SendOptions,
    ) -> Result<Signature, ChainError> {
        self.send_instructions(instructions, fee_payer, extra_signers, options, None)
    }

    fn send_instructions(
        &self,
        mut instructions: Vec<Instruction>,
        fee_payer: &dyn Signer,
        extra_signers: &[&dyn Signer],
        options: SendOptions,
        native_budget: Option<NativeSolBudget>,
    ) -> Result<Signature, ChainError> {
        if let Some(limit) = options.compute_unit_limit {
            instructions.insert(
                0,
                solana_compute_budget_interface::ComputeBudgetInstruction::set_compute_unit_limit(
                    limit,
                ),
            );
        }
        let recent = self.rpc.get_latest_blockhash()?;

        let mut signers: Vec<&dyn Signer> = Vec::with_capacity(1 + extra_signers.len());
        signers.push(fee_payer);
        for signer in extra_signers {
            if signer.pubkey() != fee_payer.pubkey() {
                signers.push(*signer);
            }
        }

        let mut tx = Transaction::new_with_payer(&instructions, Some(&fee_payer.pubkey()));
        tx.message.recent_blockhash = recent;
        if let Some(budget) = native_budget {
            let fee_lamports = if fee_payer.pubkey() == budget.owner {
                self.rpc.get_fee_for_message(&tx.message)?
            } else {
                0
            };
            let available_lamports = self.rpc.get_balance(&budget.owner)?;
            budget.validate(available_lamports, fee_lamports)?;
        }
        tx.try_sign(&signers, recent)?;
        self.rpc
            .send_and_confirm_transaction_with_spinner_and_commitment(&tx, self.commitment)
            .map_err(ChainError::from)
    }
}

#[cfg(test)]
fn build_signed_transaction(
    instructions: Vec<Instruction>,
    fee_payer: Pubkey,
    signers: &[&dyn Signer],
    recent_blockhash: solana_sdk::hash::Hash,
) -> Result<Transaction, ChainError> {
    let mut transaction = Transaction::new_with_payer(&instructions, Some(&fee_payer));
    transaction.try_sign(signers, recent_blockhash)?;
    Ok(transaction)
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

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
