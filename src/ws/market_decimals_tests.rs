use super::*;

#[test]
fn descriptors_response_fills_decimals_by_market_pda() {
    let message: ServerMessage = serde_json::from_value(serde_json::json!({
        "type": "MarketDescriptors",
        "data": {
            "request_id": "00000000-0000-0000-0000-000000000099",
            "markets": [{
                "market": {
                    "chain_id": 0,
                    "program_id": "2PEfSGdYo17syw6vewKJceXbKHV481KWjXbo1x6g9E9D",
                    "market_pda": "GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9",
                    "underlying_mint": "opdhLMLuFCTUEq3YCwdtgqfuhtMqdkHBsJf6YrHfUFF",
                    "quote_mint": "Aa8SVbHR7yKKA6Zq2M2Yw9jGvNsZkMD6QKwCCCR7vgeA",
                    "expiry_ts": 1_700_000_000,
                    "is_put": false,
                    "collateral_mint": "opdhLMLuFCTUEq3YCwdtgqfuhtMqdkHBsJf6YrHfUFF",
                    "settlement_mint": "Aa8SVbHR7yKKA6Zq2M2Yw9jGvNsZkMD6QKwCCCR7vgeA"
                },
                "products": ["covered_call", "cash_secured_put", "call_spread", "put_spread"],
                "underlying_oracle_pda": "8C1MNgezngK1mYmZL5rdxTfQDFhhfAcJajjN7soBWCCn",
                "quote_oracle_pda": "5E7ARf86php4DmyzznNjNqHJVaNMzUG83bvNK9aXw6Ge",
                "underlying_decimals": 9,
                "quote_decimals": 6,
                "size_rule": { "min_size": 1, "max_size": 100, "step": 1 },
                "underlying_symbol": "SOL",
                "quote_symbol": "USDC"
            }]
        }
    }))
    .unwrap();

    let mut decimals = MarketDecimals::new();
    decimals.observe(&message);

    assert_eq!(
        decimals.get("GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9"),
        Some(TokenDecimals {
            underlying: Decimals::new(9),
            quote: Decimals::new(6),
        })
    );
    assert_eq!(decimals.get("other"), None);
}
