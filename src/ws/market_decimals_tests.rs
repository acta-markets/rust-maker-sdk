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
                    "program_id": "p",
                    "market_pda": "market",
                    "underlying_mint": "u",
                    "quote_mint": "q",
                    "expiry_ts": 1_700_000_000,
                    "is_put": false,
                    "collateral_mint": "u",
                    "settlement_mint": "q"
                },
                "underlying_oracle_pda": "uo",
                "quote_oracle_pda": "qo",
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
        decimals.get("market"),
        Some(TokenDecimals {
            underlying: Decimals::new(9),
            quote: Decimals::new(6),
        })
    );
    assert_eq!(decimals.get("other"), None);
}
