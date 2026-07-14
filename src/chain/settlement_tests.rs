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
}
