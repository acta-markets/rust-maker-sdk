use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::ix::{
    SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
    derive_associated_token_address as ata, maker_pda_with_program_id,
};
use super::{ChainError, MarketInfo, PositionInfo, PositionStatus};
use crate::orders::{
    SpreadOrderPreimageArgs, compute_spread_order_id, verify_order_id_signature_bytes,
};
use crate::spreads::SpreadType;

pub struct OpenSpreadIxArgs<'a> {
    pub order: &'a SpreadOrderPreimageArgs,
    pub market: &'a MarketInfo,
    pub keeper: Pubkey,
    pub quote_signer: [u8; 32],
    pub signature: [u8; 64],
}

/// Keep the signature verification immediately before OpenSpread.
pub fn build_open_spread_ixs(args: &OpenSpreadIxArgs<'_>) -> Result<Vec<Instruction>, ChainError> {
    let order = args.order;
    let market = args.market;
    if order.side.is_taker_buy() {
        return Err(ChainError::TakerBuyUnsupported);
    }
    let program_id = Pubkey::new_from_array(order.program_id);
    let market_pda = crate::quote::market_pda(
        &program_id,
        &market.underlying_mint,
        &market.quote_mint,
        market.expiry_ts,
        market.is_put,
    );
    if order.chain_id != 0
        || order.market != market_pda.to_bytes()
        || market.is_put != (order.position_type == SpreadType::PutSpread)
        || market.is_finalized
        || order.valid_until == 0
        || order.valid_until > market.expiry_ts
    {
        return Err(ChainError::InvalidAccountData);
    }
    validate_token_programs(market)?;
    crate::spreads::SpreadTerms {
        kind: order.position_type,
        lower_strike: order.lower_strike,
        upper_strike: order.upper_strike,
        quantity: order.quantity,
        quote_decimals: market.quote_decimals.value(),
        underlying_decimals: market.underlying_decimals.value(),
    }
    .premium(order.gross_price, 0, 0)?;
    let order_id = compute_spread_order_id(order)?;
    verify_order_id_signature_bytes(&order_id, &args.signature, &args.quote_signer)
        .map_err(|_| ChainError::InvalidAccountData)?;
    let maker = Pubkey::new_from_array(order.maker);
    let taker = Pubkey::new_from_array(order.taker);
    let maker_pda = maker_pda_with_program_id(&program_id, &maker).0;
    let position = Pubkey::find_program_address(&[b"position", &order_id], &program_id).0;
    let config =
        Pubkey::find_program_address(&[b"config", market.quote_mint.as_ref()], &program_id).0;
    let global = Pubkey::find_program_address(&[b"global_config"], &program_id).0;
    let quote_ata = |owner: &Pubkey| ata(owner, &market.quote_mint, &market.quote_token_program_id);
    let mut data = vec![77, order.position_type as u8];
    for value in [
        order.lower_strike,
        order.quantity,
        order.gross_price,
        order.valid_until,
        order.nonce,
    ] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    data.extend_from_slice(&order_id);
    data.extend_from_slice(&order.upper_strike.to_le_bytes());
    let open = Instruction {
        program_id,
        data,
        accounts: vec![
            AccountMeta::new(args.keeper, true),
            AccountMeta::new(taker, true),
            AccountMeta::new(quote_ata(&taker), false),
            AccountMeta::new(quote_ata(&taker), false),
            AccountMeta::new_readonly(maker_pda, false),
            AccountMeta::new(quote_ata(&maker_pda), false),
            AccountMeta::new(market_pda, false),
            AccountMeta::new(position, false),
            AccountMeta::new(quote_ata(&position), false),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(quote_ata(&config), false),
            AccountMeta::new_readonly(solana_sdk::sysvar::instructions::ID, false),
            AccountMeta::new_readonly(market.underlying_token_program_id, false),
            AccountMeta::new_readonly(market.quote_token_program_id, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(global, false),
        ],
    };
    let mut signature = vec![1, 0];
    for offset in [16u16, u16::MAX, 80, u16::MAX, 112, 32, u16::MAX] {
        signature.extend_from_slice(&offset.to_le_bytes());
    }
    signature.extend_from_slice(&args.signature);
    signature.extend_from_slice(&args.quote_signer);
    signature.extend_from_slice(&order_id);
    Ok(vec![
        Instruction {
            program_id: solana_sdk::pubkey!("Ed25519SigVerify111111111111111111111111111"),
            accounts: vec![],
            data: signature,
        },
        open,
    ])
}

#[derive(Debug, Clone, Copy)]
pub struct CashSettleDestinations {
    pub taker: Pubkey,
    pub maker: Pubkey,
}

pub fn build_cash_settle_ix(
    program_id: Pubkey,
    settler: Pubkey,
    position: &PositionInfo,
    market: &MarketInfo,
) -> Result<Instruction, ChainError> {
    let quote_ata = |owner: &Pubkey| ata(owner, &market.quote_mint, &market.quote_token_program_id);
    build_cash_settle_ix_with_destinations(
        program_id,
        settler,
        position,
        market,
        CashSettleDestinations {
            taker: quote_ata(&position.taker_owner),
            maker: quote_ata(&position.maker_owner),
        },
    )
}

pub fn build_cash_settle_ix_with_destinations(
    program_id: Pubkey,
    settler: Pubkey,
    position: &PositionInfo,
    market: &MarketInfo,
    destinations: CashSettleDestinations,
) -> Result<Instruction, ChainError> {
    if position.side.is_taker_buy() {
        return Err(ChainError::TakerBuyUnsupported);
    }
    let kind = SpreadType::try_from(u8::from(position.terms.to_chain().position_type()))?;
    if position.flags & !3 != 0
        || position.flags2 != 0
        || position.flags3 != 0
        || position.status != PositionStatus::Open
        || !market.is_finalized
        || market.is_put != (kind == SpreadType::PutSpread)
    {
        return Err(ChainError::InvalidSettlementState);
    }
    let upper = position
        .terms
        .upper_strike()
        .ok_or(ChainError::InvalidAccountData)?
        .value();
    crate::spreads::width(position.terms.to_chain().strike().value(), upper)?;
    validate_token_programs(market)?;
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
    let position_pda =
        Pubkey::find_program_address(&[b"position", position.order_id.as_bytes()], &program_id).0;
    let quote_ata = |owner: &Pubkey| ata(owner, &market.quote_mint, &market.quote_token_program_id);
    // One owner on both sides may receive both amounts into one account.
    let shared =
        position.taker_owner == position.maker_owner && destinations.taker == destinations.maker;
    let unique = [
        position_pda,
        market_pda,
        quote_ata(&position_pda),
        SYSTEM_PROGRAM_ID,
        destinations.taker,
        destinations.maker,
    ];
    let unique = &unique[..unique.len() - usize::from(shared)];
    for (index, key) in unique.iter().enumerate() {
        if unique[..index].contains(key) {
            return Err(ChainError::Identity("distinct cash settlement accounts"));
        }
    }
    // A vault side settles only into its vault's canonical quote ATA, with the vault tail.
    let vaults: Vec<Pubkey> = [
        (1, position.taker_owner, destinations.taker),
        (2, position.maker_owner, destinations.maker),
    ]
    .into_iter()
    .filter(|(flag, ..)| position.flags & flag != 0)
    .map(|(_, owner, destination)| {
        if destination == quote_ata(&owner) {
            Ok(owner)
        } else {
            Err(ChainError::Identity(
                "vault spread destination must be the vault quote ATA",
            ))
        }
    })
    .collect::<Result<_, _>>()?;
    let tail = if vaults.is_empty() {
        Vec::new()
    } else {
        let mut tail = vec![
            AccountMeta::new_readonly(market.underlying_mint, false),
            AccountMeta::new_readonly(market.quote_mint, false),
        ];
        tail.extend(vaults.iter().map(|vault| AccountMeta::new(*vault, false)));
        tail
    };
    Ok(Instruction {
        program_id,
        data: vec![10, 0],
        accounts: vec![
            AccountMeta::new(settler, true),
            AccountMeta::new(position_pda, false),
            AccountMeta::new(market_pda, false),
            AccountMeta::new(quote_ata(&position_pda), false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new(destinations.taker, false),
            AccountMeta::new(destinations.maker, false),
            AccountMeta::new_readonly(market.underlying_token_program_id, false),
            AccountMeta::new_readonly(market.quote_token_program_id, false),
        ]
        .into_iter()
        .chain(tail)
        .collect(),
    })
}

pub fn cash_settle_sequence(
    program_id: Pubkey,
    settler: Pubkey,
    position: &PositionInfo,
    market: &MarketInfo,
) -> Result<Vec<Instruction>, ChainError> {
    let settle = build_cash_settle_ix(program_id, settler, position, market)?;
    let mut ixs = Vec::with_capacity(3);
    for owner in [position.taker_owner, position.maker_owner] {
        let destination = ata(&owner, &market.quote_mint, &market.quote_token_program_id);
        ixs.push(super::ix::create_associated_token_account_idempotent_ix(
            &settler,
            &destination,
            &owner,
            &market.quote_mint,
            &market.quote_token_program_id,
        ));
    }
    ixs.push(settle);
    Ok(ixs)
}

fn validate_token_programs(market: &MarketInfo) -> Result<(), ChainError> {
    for program in [
        market.underlying_token_program_id,
        market.quote_token_program_id,
    ] {
        if program != TOKEN_PROGRAM_ID && program != TOKEN_2022_PROGRAM_ID {
            return Err(ChainError::InvalidAccountData);
        }
    }
    Ok(())
}
