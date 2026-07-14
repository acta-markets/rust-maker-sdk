use acta_maker_sdk::{PRICE_SCALE, Price};

#[test]
fn price_fee_matches_contract_rounding() {
    assert_eq!(
        Price::new(PRICE_SCALE).after_fee_bps(100),
        Price::new(990_000_000)
    );
    assert_eq!(Price::new(1).after_fee_bps(1), Price::new(1));
}

#[test]
fn price_fee_saturates_when_basis_points_exceed_one_hundred_percent() {
    assert_eq!(Price::new(10).after_fee_bps(u16::MAX), Price::new(0));
}
