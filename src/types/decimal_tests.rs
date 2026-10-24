use std::str::FromStr;

use rust_decimal::Decimal;

use crate::types::{Decimals, Price, Quantity, Strike};

fn d(value: &str) -> Decimal {
    Decimal::from_str(value).expect("decimal literal")
}

#[test]
fn scaled_values_convert_exactly() {
    assert_eq!(Strike::new(185_250_000_000).to_decimal(), d("185.25"));
    assert_eq!(Price::new(1).to_decimal(), d("0.000000001"));
    assert_eq!(
        Price::new(u64::MAX).to_decimal(),
        d("18446744073.709551615")
    );
}

#[test]
fn price_rounds_down_to_the_scaled_grid() {
    assert_eq!(
        Price::from_decimal_floor(d("1.2345678919")),
        Some(Price::new(1_234_567_891))
    );
    assert_eq!(Price::from_decimal_floor(d("0")), Some(Price::new(0)));
    assert_eq!(Price::from_decimal_floor(d("-0.000000001")), None);
    assert_eq!(Price::from_decimal_floor(d("18446744073.709551616")), None);
    let price = Price::new(42_000_000_123);
    assert_eq!(Price::from_decimal_floor(price.to_decimal()), Some(price));
}

#[test]
fn quantity_converts_with_mint_decimals() {
    let sol = Decimals::new(9);
    assert_eq!(Quantity::new(1_500_000_000).to_decimal(sol), Some(d("1.5")));
    assert_eq!(
        Quantity::from_decimal_floor(d("1.5000000009"), sol),
        Some(Quantity::new(1_500_000_000))
    );
    assert_eq!(Quantity::from_decimal_floor(d("-1"), sol), None);
    assert_eq!(Quantity::new(1).to_decimal(Decimals::new(29)), None);
    assert_eq!(
        Quantity::from_decimal_floor(d("1"), Decimals::new(20)),
        None
    );
    assert_eq!(
        Quantity::from_decimal_floor(d("0.1"), Decimals::new(20)),
        Some(Quantity::new(10_000_000_000_000_000_000))
    );
    assert_eq!(
        Quantity::from_decimal_floor(d("0"), Decimals::new(29)),
        None
    );
}
