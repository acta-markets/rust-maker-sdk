use super::*;

/// Rows of the contract's `quote_amount_and_sdk_cover_exact_range_and_rounding_boundaries`.
#[test]
fn scaled_quote_amount_matches_contract_rounding() {
    for (per_unit, quantity, quote_dec, underlying_dec, expected) in [
        (2_000_000_000, 1_000_000_000, 6, 9, Some(2_000_000)),
        (1_500_000_000, 10_000_000, 6, 6, Some(15_000_000)),
        (250_000_000, 4_000_000, 9, 6, Some(1_000_000_000)),
        (10_000_000, 3_500_000_000, 6, 9, Some(35_000)),
        (1, 1, 6, 9, Some(0)),
        (
            1_000_000_000_000,
            1_000_000_000,
            18,
            18,
            Some(1_000_000_000_000),
        ),
        (
            1_000_000_000_001,
            1_000_000_001,
            18,
            18,
            Some(1_000_000_001_001),
        ),
        (PRICE_SCALE, u64::MAX, 18, 18, Some(u64::MAX)),
        (u64::MAX, PRICE_SCALE + 1, 18, 18, None),
        (u64::MAX, u64::MAX, 18, 0, None),
        (u64::MAX, u64::MAX, 0, 18, Some(340_282_366_920)),
        (2, 3, 18, 6, Some(6_000)),
        (3, 7, 18, 9, Some(21)),
        (1, 1, 0, 18, Some(0)),
        (0, u64::MAX, 18, 0, Some(0)),
        (PRICE_SCALE, 1, 19, 0, None),
        (PRICE_SCALE, 1, 0, 19, None),
    ] {
        assert_eq!(
            scaled_quote_amount(per_unit, quantity, quote_dec, underlying_dec),
            expected,
            "{per_unit} x {quantity} @ quote {quote_dec} / underlying {underlying_dec}"
        );
    }
}

#[test]
fn total_premium_takes_underlying_then_quote_decimals() {
    // 0.05 USDC per SOL on 3.5 SOL = 0.175 USDC.
    assert_eq!(
        total_premium_quote_atoms(
            Price::new(50_000_000),
            Quantity::new(3_500_000_000),
            Decimals::new(9),
            Decimals::new(6),
        ),
        Some(175_000)
    );
}

#[test]
fn physical_fee_is_the_smaller_of_premium_and_volume_fees() {
    let split = |premium_bps, volume_bps| {
        physical_premium(
            Price::new(50_000_000),
            Strike::new(100_000_000_000),
            Quantity::new(3_500_000_000),
            Decimals::new(9),
            Decimals::new(6),
            premium_bps,
            volume_bps,
        )
    };
    // 0.5% of 0.175 USDC = 875 atoms; 0.1% of the 350 USDC notional = 350 000.
    assert_eq!(
        split(50, 10),
        Some(PhysicalPremium {
            gross_premium: 175_000,
            protocol_fee: 875,
            net_premium: 174_125,
        })
    );
    assert_eq!(split(50, 0).map(|split| split.protocol_fee), Some(0));
    assert_eq!(split(10_001, 10), None);
}

#[test]
fn display_views_divide_by_their_scale() {
    assert!((Strike::new(86_500_000_000).as_decimal() - 86.5).abs() < f64::EPSILON);
    assert!((Price::new(1_250_000_000).as_decimal() - 1.25).abs() < f64::EPSILON);
    assert!((Quantity::new(3_500_000_000).to_units(Decimals::new(9)) - 3.5).abs() < f64::EPSILON);
    assert!((Quantity::new(42).to_units(Decimals::new(0)) - 42.0).abs() < f64::EPSILON);
}
