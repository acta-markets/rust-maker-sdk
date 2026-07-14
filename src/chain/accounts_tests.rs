use super::*;

fn account(owner: Pubkey, data: Vec<u8>) -> Account {
    Account {
        lamports: 0,
        data,
        owner,
        executable: false,
        rent_epoch: 0,
    }
}

#[test]
fn position_layout_decodes_canonical_offsets() {
    let program_id = Pubkey::new_unique();
    let taker = Pubkey::new_unique();
    let maker = Pubkey::new_unique();
    let market = Pubkey::new_unique();
    let mut data = vec![0; POSITION_MIN_LEN];
    data[ACCOUNT_OFFSET_DISCRIMINATOR] = POSITION_DISCRIMINATOR;
    data[ACCOUNT_OFFSET_VERSION] = 1;
    data[POSITION_OFFSET_POSITION_TYPE] = 1;
    data[POSITION_OFFSET_STATUS] = PositionStatus::Open as u8;
    data[POSITION_OFFSET_TAKER_OWNER..POSITION_OFFSET_TAKER_OWNER + 32]
        .copy_from_slice(taker.as_ref());
    data[POSITION_OFFSET_MAKER_OWNER..POSITION_OFFSET_MAKER_OWNER + 32]
        .copy_from_slice(maker.as_ref());
    data[POSITION_OFFSET_MARKET..POSITION_OFFSET_MARKET + 32].copy_from_slice(market.as_ref());
    data[POSITION_OFFSET_STRIKE..POSITION_OFFSET_STRIKE + 8].copy_from_slice(&7u64.to_le_bytes());
    data[POSITION_OFFSET_QUANTITY..POSITION_OFFSET_QUANTITY + 8]
        .copy_from_slice(&9u64.to_le_bytes());
    data[POSITION_OFFSET_ORDER_ID..POSITION_OFFSET_ORDER_ID + 32].fill(11);

    let position = parse_position(&account(program_id, data), &program_id).unwrap();
    assert_eq!(position.position_type, PositionType::CashSecuredPut);
    assert_eq!(position.status, PositionStatus::Open);
    assert_eq!(position.taker_owner, taker);
    assert_eq!(position.maker_owner, maker);
    assert_eq!(position.market_pda, market);
    assert_eq!(position.strike, Strike::new(7));
    assert_eq!(position.quantity, Quantity::new(9));
    assert_eq!(position.order_id, OrderId::new([11; 32]));
}

#[test]
fn account_codecs_reject_truncated_or_unknown_data() {
    let program_id = Pubkey::new_unique();
    let mut truncated = vec![0; POSITION_MIN_LEN - 1];
    truncated[ACCOUNT_OFFSET_DISCRIMINATOR] = POSITION_DISCRIMINATOR;
    truncated[ACCOUNT_OFFSET_VERSION] = 1;
    assert!(matches!(
        parse_position(&account(program_id, truncated), &program_id),
        Err(ChainError::InvalidAccountData)
    ));
    let mut position = vec![0; POSITION_MIN_LEN];
    position[ACCOUNT_OFFSET_DISCRIMINATOR] = POSITION_DISCRIMINATOR;
    position[ACCOUNT_OFFSET_VERSION] = 1;
    position[POSITION_OFFSET_STATUS] = 99;
    assert!(matches!(
        parse_position(&account(program_id, position), &program_id),
        Err(ChainError::UnknownPositionStatus(99))
    ));
    let mut market = vec![0; MARKET_MIN_LEN - 1];
    market[ACCOUNT_OFFSET_DISCRIMINATOR] = MARKET_DISCRIMINATOR;
    market[ACCOUNT_OFFSET_VERSION] = 1;
    assert!(matches!(
        parse_market(&account(program_id, market), &program_id),
        Err(ChainError::InvalidAccountData)
    ));
}

#[test]
fn account_codecs_reject_wrong_owner_type_and_uninitialized_data() {
    let program_id = Pubkey::new_unique();
    let wrong_owner = Pubkey::new_unique();
    let mut data = vec![0; POSITION_MIN_LEN];
    data[ACCOUNT_OFFSET_DISCRIMINATOR] = POSITION_DISCRIMINATOR;
    data[ACCOUNT_OFFSET_VERSION] = 1;

    assert!(matches!(
        parse_position(&account(wrong_owner, data.clone()), &program_id),
        Err(ChainError::AccountOwnerMismatch { .. })
    ));

    data[ACCOUNT_OFFSET_DISCRIMINATOR] = MARKET_DISCRIMINATOR;
    assert!(matches!(
        parse_position(&account(program_id, data.clone()), &program_id),
        Err(ChainError::AccountDiscriminatorMismatch { .. })
    ));

    data[ACCOUNT_OFFSET_DISCRIMINATOR] = POSITION_DISCRIMINATOR;
    data[ACCOUNT_OFFSET_VERSION] = 0;
    assert!(matches!(
        parse_position(&account(program_id, data), &program_id),
        Err(ChainError::UninitializedAccount)
    ));
}

#[test]
fn market_layout_keeps_token_programs_distinct_from_oracles() {
    let program_id = Pubkey::new_unique();
    let underlying_program = Pubkey::new_unique();
    let quote_program = Pubkey::new_unique();
    let mut data = vec![0; MARKET_MIN_LEN];
    data[ACCOUNT_OFFSET_DISCRIMINATOR] = MARKET_DISCRIMINATOR;
    data[ACCOUNT_OFFSET_VERSION] = 1;
    data[MARKET_OFFSET_UNDERLYING_DECIMALS] = 9;
    data[MARKET_OFFSET_QUOTE_DECIMALS] = 6;
    data[MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM..MARKET_OFFSET_UNDERLYING_TOKEN_PROGRAM + 32]
        .copy_from_slice(underlying_program.as_ref());
    data[MARKET_OFFSET_QUOTE_TOKEN_PROGRAM..MARKET_OFFSET_QUOTE_TOKEN_PROGRAM + 32]
        .copy_from_slice(quote_program.as_ref());

    let market = parse_market(&account(program_id, data), &program_id).unwrap();
    assert_eq!(market.underlying_decimals, Decimals::new(9));
    assert_eq!(market.quote_decimals, Decimals::new(6));
    assert_eq!(market.underlying_token_program_id, underlying_program);
    assert_eq!(market.quote_token_program_id, quote_program);
}
