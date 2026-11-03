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

/// Settles a position the maker side of: on a sell the maker holds (a vault may write); on a
/// buy the maker or a vault writes and a wallet holds. `maker_owner` signs and pays rent.
pub fn build_settle_position_ix(
    program_id: Pubkey,
    maker_owner: Pubkey,
    position: &PositionInfo,
    market: &MarketInfo,
) -> Result<Instruction, ChainError> {
    let buy = position.side.is_taker_buy();
    require_maker_side(maker_owner, position)?;
    if position.terms.is_spread() {
        return crate::chain::spreads::build_cash_settle_ix(
            program_id,
            maker_owner,
            position,
            market,
        );
    }
    if position.flags & !7 != 0 || position.flags2 != 0 || position.flags3 != 0 {
        return Err(ChainError::InvalidAccountData);
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
    let otm = is_otm(position, market);
    let open = position.status == PositionStatus::Open;
    // An Open sell that finishes in the money is liquidated, not settled.
    if position.status != PositionStatus::Funded && !(otm || buy) {
        return Err(ChainError::InvalidSettlementState);
    }
    let Destinations {
        collateral,
        settlement,
        writer,
        holder,
    } = destinations(position, market);
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
        AccountMeta::new(ata(&position.writer, &writer.0, &writer.1), false),
        if open {
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false)
        } else {
            AccountMeta::new(ata(&position.holder, &holder.0, &holder.1), false)
        },
        AccountMeta::new_readonly(market.underlying_token_program_id, false),
        AccountMeta::new_readonly(market.quote_token_program_id, false),
    ];
    if position.flags & 1 != 0 {
        accounts.extend([
            AccountMeta::new_readonly(market.underlying_mint, false),
            AccountMeta::new_readonly(market.quote_mint, false),
            AccountMeta::new(position.writer, false),
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
    if position.terms.is_spread() {
        require_maker_side(maker_owner, position)?;
        return crate::chain::spreads::cash_settle_sequence(
            program_id,
            maker_owner,
            position,
            market,
        );
    }
    let settle = build_settle_position_ix(program_id, maker_owner, position, market)?;
    let Destinations { writer, holder, .. } = destinations(position, market);
    let mut instructions = Vec::with_capacity(3);
    // Each wallet that receives tokens needs its ATA; a vault side pays into its own legs.
    let funded = position.status == PositionStatus::Funded;
    if position.flags & 1 == 0 && (funded || position.side.is_taker_buy()) {
        instructions.push(super::lifecycle::create_destination_ata(
            maker_owner,
            position.writer,
            writer.0,
            writer.1,
        ));
    }
    if funded {
        instructions.push(super::lifecycle::create_destination_ata(
            maker_owner,
            position.holder,
            holder.0,
            holder.1,
        ));
    }
    instructions.push(settle);
    Ok(instructions)
}

/// The maker side: the holder on a sell, the writer on a buy. A vault writer may be settled by
/// any signer; a vault holder settles through the vault route instead.
fn require_maker_side(maker_owner: Pubkey, position: &PositionInfo) -> Result<(), ChainError> {
    let ok = if position.side.is_taker_buy() {
        position.flags & 1 != 0 || position.writer == maker_owner
    } else {
        position.flags & 2 == 0 && position.holder == maker_owner
    };
    if ok {
        Ok(())
    } else {
        Err(ChainError::Identity("wallet maker owner"))
    }
}

fn is_otm(position: &PositionInfo, market: &MarketInfo) -> bool {
    if market.is_put {
        market.settlement_price >= position.terms.strike().value()
    } else {
        market.settlement_price <= position.terms.strike().value()
    }
}

struct Destinations {
    collateral: (Pubkey, Pubkey),
    settlement: (Pubkey, Pubkey),
    /// Mint and token program the writer receives.
    writer: (Pubkey, Pubkey),
    /// Mint and token program the holder receives when funded.
    holder: (Pubkey, Pubkey),
}

/// Mirrors settle: the writer gets collateral back when OTM or when a buy was never funded.
fn destinations(position: &PositionInfo, market: &MarketInfo) -> Destinations {
    let underlying = (market.underlying_mint, market.underlying_token_program_id);
    let quote = (market.quote_mint, market.quote_token_program_id);
    let (collateral, settlement) = if market.is_put {
        (quote, underlying)
    } else {
        (underlying, quote)
    };
    let otm = is_otm(position, market);
    let unfunded_buy = position.side.is_taker_buy() && position.status == PositionStatus::Open;
    Destinations {
        collateral,
        settlement,
        writer: if otm || unfunded_buy {
            collateral
        } else {
            settlement
        },
        holder: if otm { settlement } else { collateral },
    }
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
