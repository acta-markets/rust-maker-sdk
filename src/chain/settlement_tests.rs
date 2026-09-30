use super::*;

#[test]
fn covered_call_matches_contract_decimal_scaling() {
    assert_eq!(
        required_settlement(0, 86_000_000_000, 50_000_000, 6, 9).unwrap(),
        4_300_000
    );
}

#[test]
fn cash_secured_put_uses_underlying_quantity() {
    assert_eq!(required_settlement(1, 0, 12_345, 6, 9).unwrap(), 12_345);
}

#[test]
fn invalid_position_type_is_rejected() {
    assert!(matches!(
        required_settlement(99, 1, 1, 6, 9),
        Err(ChainError::InvalidPositionType(99))
    ));
}

#[test]
fn unsupported_decimal_scale_is_rejected_without_panicking() {
    assert!(matches!(
        required_settlement(0, 1, 1, u8::MAX, 9),
        Err(ChainError::SettlementAmountOverflow)
    ));
    assert!(matches!(
        required_settlement(0, 1, 1, 19, 0),
        Err(ChainError::SettlementAmountOverflow)
    ));
    assert!(matches!(
        required_settlement(0, 1, 1, 0, 19),
        Err(ChainError::SettlementAmountOverflow)
    ));
}

#[test]
fn covered_call_high_decimals_preserves_floor_and_output_limit() {
    assert_eq!(
        required_settlement(0, 1_000_000_000_000, 1_000_000_000, 18, 18).unwrap(),
        1_000_000_000_000
    );
    assert_eq!(
        required_settlement(0, 1_000_000_000_001, 1_000_000_001, 18, 18).unwrap(),
        1_000_000_001_001
    );
    assert_eq!(
        required_settlement(0, PRICE_SCALE, u64::MAX, 18, 18).unwrap(),
        u64::MAX
    );
    assert_eq!(required_settlement(0, 2, 3, 18, 6).unwrap(), 6_000);
    for (strike, quantity, quote, underlying) in [
        (u64::MAX, PRICE_SCALE + 1, 18, 18),
        (u64::MAX, u64::MAX, 18, 0),
    ] {
        assert!(matches!(
            required_settlement(0, strike, quantity, quote, underlying),
            Err(ChainError::SettlementAmountOverflow)
        ));
    }
}

#[test]
fn wsol_quote_funding_does_not_overflow_before_decimal_cancellation() {
    assert_eq!(
        required_settlement(0, 100_000_000_000, 10_000_000_000_000_000_000, 9, 18).unwrap(),
        1_000_000_000_000,
    );
}
