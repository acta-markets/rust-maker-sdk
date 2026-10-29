use super::pda;
use crate::chain::ix::{
    ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
    derive_associated_token_address as ata,
};
use crate::{
    PositionType,
    chain::{ChainError, MarketInfo, PositionInfo, PositionStatus},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

/// Settles a wallet maker's own position, including a vault writer.
pub fn build_settle_position_ix(
    program_id: Pubkey,
    maker_owner: Pubkey,
    position: &PositionInfo,
    market: &MarketInfo,
) -> Result<Instruction, ChainError> {
    if position.side.is_taker_buy() {
        return Err(ChainError::TakerBuyUnsupported);
    }
    if position.terms.is_spread() {
        if position.maker_owner != maker_owner {
            return Err(ChainError::Identity("wallet maker owner"));
        }
        return crate::chain::spreads::build_cash_settle_ix(
            program_id,
            maker_owner,
            position,
            market,
        );
    }
    if position.flags & !3 != 0 || position.flags2 != 0 || position.flags3 != 0 {
        return Err(ChainError::InvalidAccountData);
    }
    if position.maker_owner != maker_owner || position.flags & 2 != 0 {
        return Err(ChainError::Identity("wallet maker owner"));
    }
    let market_pda = crate::quote::market_pda(
        &program_id,
        &market.underlying_mint,
        &market.quote_mint,
        market.expiry_ts,
        market.is_put,
    );
    if market_pda != position.market_pda {
        return Err(ChainError::Identity("position market"));
    }
    if !market.is_finalized
        || market.is_put != (position.terms.position_type() == PositionType::CashSecuredPut)
    {
        return Err(ChainError::InvalidSettlementState);
    }
    for program in [
        market.underlying_token_program_id,
        market.quote_token_program_id,
    ] {
        if program != TOKEN_PROGRAM_ID && program != TOKEN_2022_PROGRAM_ID {
            return Err(ChainError::InvalidAccountData);
        }
    }
    let otm = if market.is_put {
        market.settlement_price >= position.terms.strike().value()
    } else {
        market.settlement_price <= position.terms.strike().value()
    };
    let open = position.status == PositionStatus::Open;
    if position.status != PositionStatus::Funded && !(otm && open) {
        return Err(ChainError::InvalidSettlementState);
    }
    let underlying = (market.underlying_mint, market.underlying_token_program_id);
    let quote = (market.quote_mint, market.quote_token_program_id);
    let (collateral, settlement) = if market.is_put {
        (quote, underlying)
    } else {
        (underlying, quote)
    };
    let (writer, holder) = if otm {
        (collateral, settlement)
    } else {
        (settlement, collateral)
    };
    let position_pda = pda::position(&program_id, position.order_id.as_bytes()).0;
    let mut accounts = vec![
        AccountMeta::new(maker_owner, true),
        AccountMeta::new(position_pda, false),
        AccountMeta::new(market_pda, false),
        AccountMeta::new(ata(&position_pda, &collateral.0, &collateral.1), false),
        if open {
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false)
        } else {
            AccountMeta::new(ata(&position_pda, &settlement.0, &settlement.1), false)
        },
        AccountMeta::new(ata(&position.taker_owner, &writer.0, &writer.1), false),
        if open {
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false)
        } else {
            AccountMeta::new(ata(&maker_owner, &holder.0, &holder.1), false)
        },
        AccountMeta::new_readonly(underlying.1, false),
        AccountMeta::new_readonly(quote.1, false),
    ];
    if position.flags & 1 != 0 {
        accounts.extend([
            AccountMeta::new_readonly(underlying.0, false),
            AccountMeta::new_readonly(quote.0, false),
            AccountMeta::new(position.taker_owner, false),
        ]);
    }
    Ok(Instruction {
        program_id,
        accounts,
        data: vec![10, 0],
    })
}

pub fn settle_position_sequence(
    program_id: Pubkey,
    maker_owner: Pubkey,
    position: &PositionInfo,
    market: &MarketInfo,
) -> Result<Vec<Instruction>, ChainError> {
    if position.side.is_taker_buy() {
        return Err(ChainError::TakerBuyUnsupported);
    }
    if position.terms.is_spread() {
        if position.maker_owner != maker_owner {
            return Err(ChainError::Identity("wallet maker owner"));
        }
        return crate::chain::spreads::cash_settle_sequence(
            program_id,
            maker_owner,
            position,
            market,
        );
    }
    let settle = build_settle_position_ix(program_id, maker_owner, position, market)?;
    let mut instructions = Vec::with_capacity(3);
    if position.status == PositionStatus::Funded {
        let otm = if market.is_put {
            market.settlement_price >= position.terms.strike().value()
        } else {
            market.settlement_price <= position.terms.strike().value()
        };
        let underlying = (market.underlying_mint, market.underlying_token_program_id);
        let quote = (market.quote_mint, market.quote_token_program_id);
        let (writer, holder) = if market.is_put == otm {
            (quote, underlying)
        } else {
            (underlying, quote)
        };
        if position.flags & 1 == 0 {
            instructions.push(super::lifecycle::create_destination_ata(
                maker_owner,
                position.taker_owner,
                writer.0,
                writer.1,
            ));
        }
        instructions.push(super::lifecycle::create_destination_ata(
            maker_owner,
            maker_owner,
            holder.0,
            holder.1,
        ));
    }
    instructions.push(settle);
    Ok(instructions)
}

#[cfg(feature = "chain-rpc")]
impl crate::chain::ChainClient {
    pub async fn build_settle_position(
        &self,
        maker_owner: Pubkey,
        position_pda: Pubkey,
    ) -> Result<Instruction, ChainError> {
        let (position, market) = self.settlement_snapshots(position_pda).await?;
        build_settle_position_ix(self.program_id(), maker_owner, &position, &market)
    }

    pub async fn settle_position_sequence(
        &self,
        maker_owner: Pubkey,
        position_pda: Pubkey,
    ) -> Result<Vec<Instruction>, ChainError> {
        let (position, market) = self.settlement_snapshots(position_pda).await?;
        settle_position_sequence(self.program_id(), maker_owner, &position, &market)
    }

    async fn settlement_snapshots(
        &self,
        position_pda: Pubkey,
    ) -> Result<(PositionInfo, MarketInfo), ChainError> {
        let position = self.fetch_position_info(&position_pda).await?;
        if pda::position(&self.program_id(), position.order_id.as_bytes()).0 != position_pda {
            return Err(ChainError::Identity("position PDA"));
        }
        let market = self.fetch_market_info(&position.market_pda).await?;
        Ok((position, market))
    }
}
