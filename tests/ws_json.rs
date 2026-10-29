use acta_maker_sdk::{
    Decimals, MarketId, Nonce, OptionTerms, OrderId, OrderVersion, PositionType, Price,
    QuoteExpiry, RfqVersion, Strike, TakerSide,
};
use proptest::prelude::*;

use acta_maker_sdk::ws::types::*;
use serde_json::json;
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[test]
fn auth_success_requires_expires_at() {
    for data in [
        json!({"session_id":"s"}),
        json!({"session_id":"s","expires_at":null,"maker_pda":null}),
        json!({"session_id":"s","expires_at":1}),
    ] {
        assert!(
            serde_json::from_value::<ServerMessage>(json!({"type":"AuthSuccess","data":data}))
                .is_err()
        );
    }
    let message: ServerMessage = serde_json::from_value(json!({
        "type":"AuthSuccess", "data":{"session_id":"s","expires_at":1_710_086_400,"maker_pda":null}
    }))
    .unwrap();
    let ServerMessage::AuthSuccess(data) = message else {
        panic!("expected AuthSuccess")
    };
    assert_eq!(
        data.expires_at,
        UNIX_EPOCH + Duration::from_secs(1_710_086_400)
    );
}

#[test]
fn order_lifecycle_frames_preserve_reconciliation_rank() {
    let confirmed: ServerMessage = serde_json::from_value(json!({
        "type": "OrderConfirmed",
        "data": {
            "order_id": "0x0101010101010101010101010101010101010101010101010101010101010101",
            "position_pda": "75NqraLLofiLUpNQg2wUPG6CJpxcaGEZUwuNrnAgvcTP",
            "order_version": 4
        }
    }))
    .unwrap();
    let failed: ServerMessage = serde_json::from_value(json!({
        "type": "OrderFailed",
        "data": {
            "order_id": "0x0101010101010101010101010101010101010101010101010101010101010101",
            "reason": "local failure",
            "order_version": 3
        }
    }))
    .unwrap();

    assert!(matches!(
        confirmed,
        ServerMessage::OrderConfirmed(data) if data.order_version == OrderVersion::CONFIRMED
    ));
    assert!(matches!(
        failed,
        ServerMessage::OrderFailed(data) if data.order_version == OrderVersion::FAILED
    ));
}

#[test]
fn auth_error_requires_message() {
    let raw = json!({
        "type": "AuthError",
        "data": {
            "reason": "invalid_signature",
            "message": "bad signature bytes"
        }
    });
    match serde_json::from_value::<ServerMessage>(raw).unwrap() {
        ServerMessage::AuthError(data) => {
            assert_eq!(data.reason, WsErrorCode::InvalidSignature);
            assert!(!data.is_session_expired());
            assert_eq!(data.message, "bad signature bytes");
        }
        _ => panic!("expected AuthError"),
    }
    let without_message = json!({
        "type": "AuthError",
        "data": { "reason": "session_expired" }
    });
    assert!(serde_json::from_value::<ServerMessage>(without_message).is_err());
}

enum ExpectedParse {
    QuotesUpdate {
        rfq_id: Uuid,
        quotes_len: usize,
    },
    ChainEventMakerRegistered {
        quote_signing: &'static str,
    },
    RfqClosed {
        rfq_id: Uuid,
        rfq_version: u64,
    },
    QuoteCancelled {
        order_ids_len: usize,
        reason: QuoteCancelReason,
    },
}

#[test]
fn server_message_parses_cases() {
    let rfq_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let cases = vec![
        (
            "quotes_update",
            json!({
                "type": "QuotesUpdate",
                "data": {
                    "rfq_id": rfq_id,
                    "quotes": [{
                        "rfq_id": rfq_id,
                        "terms": {"type": "covered_call", "strike": 1},
                        "side": "sell",
                        "maker": "A87ysNWAmmqQHafLSQGbYSLhtcV3BjXctR88EWuSobGP",
                        "price": 2,
                        "valid_until": 3,
                        "nonce": 4,
                        "order_id": "0000000000000000000000000000000000000000000000000000000000000002",
                        "net_price": null
                    }]
                }
            }),
            ExpectedParse::QuotesUpdate {
                rfq_id,
                quotes_len: 1,
            },
        ),
        (
            "chain_event_maker_registered",
            json!({
                "type": "ChainEvent",
                "data": {
                    "event_type": "MakerRegistered",
                    "instruction_index": 0,
                    "signature": "sig",
                    "slot": 1,
                    "owner": "67vHA8qZGCJKw1UNGUJZME4MwEWDRGWzp7MGvsut43A8",
                    "maker_pda": "AtmZsVb7FzNqY2vM7XGjLggYS5hWKPZEGJo8tRUKQyh9",
                    "quote_signing": "B2NTpbB7KZUcVJRmgYk4g6LJfNGpHiiyw98tdWhk5bMG"
                }
            }),
            ExpectedParse::ChainEventMakerRegistered {
                quote_signing: "B2NTpbB7KZUcVJRmgYk4g6LJfNGpHiiyw98tdWhk5bMG",
            },
        ),
        (
            "rfq_closed",
            json!({
                "type": "RfqClosed",
                "data": {
                    "rfq_id": rfq_id,
                    "rfq_version": 7,
                    "reason": "expired",
                    "your_quote": null,
                    "winner": null,
                    "closed_at": 123
                }
            }),
            ExpectedParse::RfqClosed {
                rfq_id,
                rfq_version: 7,
            },
        ),
        (
            "quote_cancelled",
            json!({
                "type": "QuoteCancelled",
                "data": {
                    "rfq_id": rfq_id,
                    "order_ids": [],
                    "reason": "requested",
                    "cancelled_at": 123
                }
            }),
            ExpectedParse::QuoteCancelled {
                order_ids_len: 0,
                reason: QuoteCancelReason::Requested,
            },
        ),
    ];

    for (name, raw, expected) in cases {
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        match (msg, expected) {
            (
                ServerMessage::QuotesUpdate(update),
                ExpectedParse::QuotesUpdate { rfq_id, quotes_len },
            ) => {
                assert_eq!(update.rfq_id, rfq_id, "{name}");
                assert_eq!(update.quotes.len(), quotes_len, "{name}");
            }
            (
                ServerMessage::ChainEvent(ChainEventMessage::MakerRegistered(event)),
                ExpectedParse::ChainEventMakerRegistered {
                    quote_signing: expected,
                },
            ) => {
                assert_eq!(
                    event.quote_signing,
                    acta_maker_sdk::Address32::from_base58(expected).expect("valid fixture"),
                    "{name}"
                );
            }
            (
                ServerMessage::RfqClosed(closed),
                ExpectedParse::RfqClosed {
                    rfq_id,
                    rfq_version,
                },
            ) => {
                assert_eq!(closed.rfq_id, rfq_id, "{name}");
                assert_eq!(closed.rfq_version, RfqVersion::new(rfq_version), "{name}");
            }
            (
                ServerMessage::QuoteCancelled(cancelled),
                ExpectedParse::QuoteCancelled {
                    order_ids_len,
                    reason,
                },
            ) => {
                assert_eq!(cancelled.order_ids.len(), order_ids_len, "{name}");
                assert_eq!(cancelled.reason, reason, "{name}");
            }
            _ => panic!("unexpected message for case: {name}"),
        }
    }
}

#[test]
fn market_descriptors_parses_oracle_pdas() {
    let raw = json!({
        "type": "MarketDescriptors",
        "data": {
            "request_id": "00000000-0000-0000-0000-000000000099",
            "markets": [
                {
                    "market": {
                        "chain_id": 0,
                        "program_id": "56sH1MNVmTQrgJkE5jx1n5UywSXwaRCzPk8kbDPJxtg8",
                        "market_pda": "EgEPSuJi5yUcefuRPUj1gokToN9evHXzGi1kmgAqykRo",
                        "underlying_mint": "EpYomzWUx3KVQeXMf1p63PEQY8EzA1L6gveWucfkroxZ",
                        "quote_mint": "9DUKchwovS3ronPfwVUGhkdGCYrYkm3RKiSE9tEhV2e5",
                        "expiry_ts": 1_700_000_000,
                        "is_put": false,
                        "collateral_mint": "7FgRiCzvuTYaGomveJrMmb457e6dBX8FjY58u5AGim9x",
                        "settlement_mint": "9KxUNNgte8fUJuVzCHef1Haif8MrwS89TNgfcnrdzd5"
                    },
                    "products": ["covered_call", "cash_secured_put", "call_spread", "put_spread"],
                "underlying_oracle_pda": "4AM1dLXFNGtW2SkY8XTiK8QM7NUgf33pEG2b7jkzzZkt",
                    "quote_oracle_pda": "gq8Lh5pbDsWTke3gEKYw7PpivQCaNPHWpd7UDrN3HUg",
                    "underlying_decimals": 9,
                    "quote_decimals": 6,
                    "size_rule": { "min_size": 1, "max_size": 100, "step": 1 },
                    "underlying_symbol": "SOL",
                    "quote_symbol": "USDC"
                }
            ]
        }
    });

    let msg: ServerMessage = serde_json::from_value(raw).unwrap();
    match msg {
        ServerMessage::MarketDescriptors(data) => {
            assert_eq!(data.markets.len(), 1);
            let market = &data.markets[0];
            assert_eq!(
                market.underlying_oracle_pda,
                acta_maker_sdk::Address32::from_base58(
                    "4AM1dLXFNGtW2SkY8XTiK8QM7NUgf33pEG2b7jkzzZkt"
                )
                .unwrap()
            );
            assert_eq!(
                market.quote_oracle_pda,
                acta_maker_sdk::Address32::from_base58(
                    "gq8Lh5pbDsWTke3gEKYw7PpivQCaNPHWpd7UDrN3HUg"
                )
                .unwrap()
            );
            assert_eq!(market.underlying_decimals, Decimals::new(9));
            assert_eq!(market.quote_decimals, Decimals::new(6));
        }
        _ => panic!("expected MarketDescriptors"),
    }
}

#[test]
fn server_error_market_metadata_incomplete_roundtrip() {
    let msg = ServerMessage::Error(ServerError::MarketMetadataIncomplete {
        details: "missing oracle PDAs for market".to_string(),
    });

    let raw = serde_json::to_string(&msg).unwrap();
    assert!(raw.contains("MarketMetadataIncomplete"));

    let parsed: ServerMessage = serde_json::from_str(&raw).unwrap();
    match parsed {
        ServerMessage::Error(ServerError::MarketMetadataIncomplete { details }) => {
            assert_eq!(details, "missing oracle PDAs for market");
        }
        _ => panic!("expected Error(MarketMetadataIncomplete)"),
    }
}

#[test]
fn my_active_rfqs_data_requires_request_id() {
    let request_id = Uuid::new_v4();
    let msg = ServerMessage::MyActiveRfqs(MyActiveRfqsData {
        request_id,
        rfqs: vec![],
    });
    let json = serde_json::to_string(&msg).expect("serialize payload with request id");
    assert!(json.contains("\"request_id\""));

    let parsed: ServerMessage = serde_json::from_str(&json).expect("deserialize");
    match parsed {
        ServerMessage::MyActiveRfqs(data) => assert_eq!(data.request_id, request_id),
        _ => panic!("expected MyActiveRfqs"),
    }

    let missing_request_id = r#"{"type":"MyActiveRfqs","data":{"rfqs":[]}}"#;
    let err = serde_json::from_str::<ServerMessage>(missing_request_id)
        .expect_err("request_id must be mandatory");
    assert!(err.to_string().contains("request_id"));
}

#[test]
fn positions_response_requires_request_id() {
    let request_id = Uuid::new_v4();
    let msg = ServerMessage::Positions(PositionsData {
        request_id,
        positions: vec![],
    });
    let json = serde_json::to_string(&msg).expect("serialize positions");
    assert!(json.contains("\"request_id\""));

    let missing_request_id = r#"{"type":"Positions","data":{"positions":[]}}"#;
    let err = serde_json::from_str::<ServerMessage>(missing_request_id)
        .expect_err("request_id must be mandatory");
    assert!(err.to_string().contains("request_id"));
}

#[test]
fn indicative_prices_response_requires_request_id() {
    use acta_maker_sdk::PositionType;

    let request_id = Uuid::new_v4();
    let msg = ServerMessage::IndicativePrices(IndicativePricesMessage {
        request_id,
        market: acta_maker_sdk::MarketId::new(acta_maker_sdk::Address32::new([
            228, 214, 147, 210, 78, 100, 108, 183, 135, 132, 38, 34, 93, 144, 210, 96, 127, 212,
            14, 46, 124, 130, 244, 207, 130, 70, 178, 204, 62, 13, 236, 184,
        ])),
        position_type: PositionType::CoveredCall,
        updated_at: UNIX_EPOCH,
        is_stale: false,
        strikes: vec![],
    });
    let json = serde_json::to_string(&msg).expect("serialize indicative prices");
    assert!(json.contains("\"request_id\""));

    let missing_request_id = r#"{"type":"IndicativePrices","data":{"market":"GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9","position_type":"covered_call","updated_at":0,"is_stale":false,"strikes":[]}}"#;
    let err = serde_json::from_str::<ServerMessage>(missing_request_id)
        .expect_err("request_id must be mandatory");
    assert!(err.to_string().contains("request_id"));
}

#[test]
fn get_my_active_rfqs_roundtrip() {
    let request_id = Uuid::new_v4();
    let msg = ClientMessage::GetMyActiveRfqs(GetMyActiveRfqsMessage { request_id });
    let json = serde_json::to_string(&msg).expect("serialize request");

    let parsed: ClientMessage = serde_json::from_str(&json).expect("deserialize request");
    match parsed {
        ClientMessage::GetMyActiveRfqs(data) => assert_eq!(data.request_id, request_id),
        _ => panic!("expected GetMyActiveRfqs"),
    }
}

#[test]
fn get_positions_requires_request_id() {
    let request_id = Uuid::new_v4();
    let msg = ClientMessage::GetPositions(GetPositionsMessage {
        request_id,
        ..Default::default()
    });
    let json = serde_json::to_string(&msg).expect("serialize request");

    let parsed: ClientMessage = serde_json::from_str(&json).expect("deserialize request");
    match parsed {
        ClientMessage::GetPositions(data) => assert_eq!(data.request_id, request_id),
        _ => panic!("expected GetPositions"),
    }
}

#[test]
fn cancel_quote_roundtrip_includes_request_id() {
    let request_id = Uuid::new_v4();
    let rfq_id = Uuid::new_v4();
    let msg = ClientMessage::CancelQuote(CancelQuoteData { rfq_id, request_id });
    let json = serde_json::to_string(&msg).expect("serialize cancel quote");

    let parsed: ClientMessage = serde_json::from_str(&json).expect("deserialize cancel quote");
    match parsed {
        ClientMessage::CancelQuote(data) => {
            assert_eq!(data.rfq_id, rfq_id);
            assert_eq!(data.request_id, request_id);
        }
        _ => panic!("expected CancelQuote"),
    }
}

#[test]
fn welcome_roundtrip() {
    let msg = ServerMessage::Welcome(WelcomeData {
        protocol_version: "1.0.0".to_string(),
        server_version: "0.5.0".to_string(),
        min_supported_version: "1.0.0".to_string(),

        server_time_unix_ms: UNIX_EPOCH + Duration::from_millis(1_700_000_000_000),
        min_quote_validity_seconds: 100,
        settlement_buffer_seconds: 90,
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::Welcome(data) => {
            assert_eq!(data.protocol_version, "1.0.0");
            assert_eq!(
                data.server_time_unix_ms,
                UNIX_EPOCH + Duration::from_millis(1_700_000_000_000)
            );
        }
        _ => panic!("expected Welcome"),
    }
}

#[test]
fn quote_acknowledged_roundtrip() {
    let rfq_id = Uuid::new_v4();
    let order_id = OrderId::new([1u8; 32]);
    let replaced = OrderId::new([2u8; 32]);
    let msg = ServerMessage::QuoteAcknowledged(QuoteAcknowledgedMessage {
        rfq_id,
        order_id,
        replaced_order_id: Some(replaced),
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::QuoteAcknowledged(data) => {
            assert_eq!(data.rfq_id, rfq_id);
            assert_eq!(data.order_id, order_id);
            assert_eq!(data.replaced_order_id, Some(replaced));
        }
        _ => panic!("expected QuoteAcknowledged"),
    }
}

#[test]
fn quote_filled_names_its_market() {
    let frame = json!({
        "type": "QuoteFilled",
        "data": {
            "rfq_id": "00000000-0000-0000-0000-000000000001",
            "order_id": "01".repeat(32),
            "market_pda": "GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9",
            "expiry_ts": 1_700_000_000,
            "terms": {"type": "cash_secured_put", "strike": 86_000_000_000_u64},
            "side": "sell",
            "taker": "5DNHbugre3SXXogpZroA5SEsENdAd69AfNK5Z7ZCWr84",
            "price": 50_000_000,
            "quantity": 3_500_000_000_u64,
            "position_pda": "75NqraLLofiLUpNQg2wUPG6CJpxcaGEZUwuNrnAgvcTP",
            "tx_signature": "sig",
            "filled_at": 1_699_999_000,
        },
    });
    let ServerMessage::QuoteFilled(filled) = serde_json::from_value(frame.clone()).unwrap() else {
        panic!("expected QuoteFilled");
    };
    assert_eq!(
        filled.market_pda,
        acta_maker_sdk::Address32::from_base58("GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9")
            .unwrap()
    );
    assert_eq!(
        filled.expiry_ts,
        Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000))
    );
    assert_eq!(filled.terms.position_type(), PositionType::CashSecuredPut);
    assert_eq!(
        serde_json::to_value(ServerMessage::QuoteFilled(filled)).unwrap(),
        frame
    );
    for field in ["market_pda", "terms", "side"] {
        let mut missing = frame.clone();
        missing["data"].as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<ServerMessage>(missing).is_err(),
            "{field}"
        );
    }
    let mut unknown_expiry = frame.clone();
    unknown_expiry["data"]["expiry_ts"] = serde_json::Value::Null;
    let ServerMessage::QuoteFilled(filled) =
        serde_json::from_value(unknown_expiry.clone()).unwrap()
    else {
        panic!("expected QuoteFilled");
    };
    assert_eq!(filled.expiry_ts, None);
    assert_eq!(
        serde_json::to_value(ServerMessage::QuoteFilled(filled)).unwrap(),
        unknown_expiry
    );
}

#[test]
fn rfq_broadcast_roundtrip() {
    use acta_maker_sdk::Quantity;

    let rfq_id = Uuid::new_v4();
    let msg = ServerMessage::RfqBroadcast(RfqBroadcastMessage {
        rfq_id,
        market: MarketDescriptor {
            chain_id: acta_maker_sdk::ChainId::new(0),
            program_id: acta_maker_sdk::Address32::new([61; 32]),
            market_pda: acta_maker_sdk::Address32::from_base58(
                "GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9",
            )
            .unwrap(),
            underlying_mint: acta_maker_sdk::Address32::new([52; 32]),
            quote_mint: acta_maker_sdk::Address32::new([99; 32]),
            expiry_ts: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            is_put: false,
            collateral_mint: acta_maker_sdk::Address32::new([85; 32]),
            settlement_mint: acta_maker_sdk::Address32::new([167; 32]),
        },
        terms: OptionTerms::CoveredCall {
            strike: Strike::new(100),
        },
        side: TakerSide::Sell,
        quantity: Quantity::new(10),
        expires_at: UNIX_EPOCH + Duration::from_secs(1_700_000_100),
        taker: acta_maker_sdk::Address32::new([60; 32]),
        sent_at_unix_ms: UNIX_EPOCH + Duration::from_millis(1_700_000_099_250),
        order_options: vec![OptionTerms::CoveredCall {
            strike: Strike::new(100),
        }],
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::RfqBroadcast(data) => {
            assert_eq!(data.rfq_id, rfq_id);
            assert_eq!(data.terms.strike(), Strike::new(100));
            assert_eq!(data.order_options.len(), 1);
            assert_eq!(
                data.sent_at_unix_ms,
                UNIX_EPOCH + Duration::from_millis(1_700_000_099_250)
            );
        }
        _ => panic!("expected RfqBroadcast"),
    }
}

#[test]
fn rfq_broadcast_requires_sent_at() {
    let raw = r#"{"type":"RfqBroadcast","data":{
        "rfq_id":"6f0a4b6e-0000-4000-8000-000000000001",
        "market":{"chain_id":0,"program_id":"2PEfSGdYo17syw6vewKJceXbKHV481KWjXbo1x6g9E9D","market_pda":"7eaSCSQv2PN6eFnRhv8ozsYN89yoPY7GKZUVa7DSDfqs","underlying_mint":"opdhLMLuFCTUEq3YCwdtgqfuhtMqdkHBsJf6YrHfUFF",
                  "quote_mint":"Aa8SVbHR7yKKA6Zq2M2Yw9jGvNsZkMD6QKwCCCR7vgeA","expiry_ts":1700000000,"is_put":false,
                  "collateral_mint":"48UTVdfmJax7bRvUZK4PwPYFmRz4PPU4PzJez1e26baD","settlement_mint":"HWKURC2rEgX95mP3DiGiGZcSeHYwJCwPMwPPy7TViv4"},
        "terms":{"type":"covered_call","strike":100},"side":"sell","quantity":10,
        "expires_at":1700000100,"taker":"GKwe8PT8YLdJiztcH7TpVkf7GVHFtHu4tNW9x5woDp6T","order_options":[]}}"#;
    assert!(parse_server_message(raw).is_err());
}

#[test]
fn server_error_variants_roundtrip() {
    let cases: Vec<ServerError> = vec![
        ServerError::RfqNotFound,
        ServerError::RfqNotActive,
        ServerError::QuoteNotFound,
        ServerError::QuoteExpired,
        ServerError::InternalError,
        ServerError::Cap(CapError::MakerPositionCapExceeded {
            current: 5,
            limit: 3,
        }),
        ServerError::Generic {
            code: WsErrorCode::TradingPaused,
            message: "test error".to_string(),
        },
    ];

    for error in cases {
        let msg = ServerMessage::Error(error);
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, ServerMessage::Error(_)));
    }
}

#[test]
fn quote_message_roundtrip() {
    let rfq_id = Uuid::new_v4();
    let order_id = OrderId::new([3u8; 32]);
    let msg = ClientMessage::Quote(QuoteMessage {
        rfq_id,
        terms: OptionTerms::CoveredCall {
            strike: Strike::new(50),
        },
        side: TakerSide::Sell,
        price: Price::new(100),
        valid_until: QuoteExpiry::from_unix_seconds(999),
        nonce: Nonce::new(42),
        order_id,
        signature: "base58sig".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ClientMessage::Quote(data) => {
            assert_eq!(data.rfq_id, rfq_id);
            assert_eq!(data.terms.strike(), Strike::new(50));
            assert_eq!(data.price, Price::new(100));
            assert_eq!(data.nonce, Nonce::new(42));
            assert_eq!(data.order_id, order_id);
            assert_eq!(data.signature, "base58sig");
        }
        _ => panic!("expected Quote"),
    }
}

#[test]
fn terms_and_side_are_required_and_terms_are_closed_tagged_unions() {
    let request = json!({
        "type": "RfqRequest",
        "data": {
            "request_id": Uuid::nil(),
            "market": "GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9",
            "terms": {
                "type": "call_spread",
                "lower_strike": 100,
                "upper_strike": 120,
            },
            "side": "sell",
            "quantity": 1,
            "timeout_seconds": 30,
        }
    });
    let parsed = serde_json::from_value::<ClientMessage>(request.clone())
        .expect("strict spread request parses");
    assert_eq!(serde_json::to_value(parsed).expect("round trips"), request);

    for field in ["terms", "side"] {
        let mut missing = request.clone();
        missing["data"]
            .as_object_mut()
            .expect("object")
            .remove(field);
        assert!(
            serde_json::from_value::<ClientMessage>(missing).is_err(),
            "{field} must be required"
        );
    }

    for invalid_terms in [
        json!({"type": "covered_call", "strike": 100, "upper_strike": 120}),
        json!({"type": "call_spread", "strike": 100, "upper_strike": 120}),
    ] {
        let mut invalid = request.clone();
        invalid["data"]["terms"] = invalid_terms;
        assert!(serde_json::from_value::<ClientMessage>(invalid).is_err());
    }
}

#[test]
fn check_quote_and_quote_received_carry_strict_terms_side_and_required_null_net_price() {
    let check = json!({
        "type": "CheckQuote",
        "data": {
            "request_id": Uuid::nil(),
            "market_id": "GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9",
            "quantity": 1,
            "price": 2,
            "terms": {
                "type": "put_spread",
                "lower_strike": 100,
                "upper_strike": 120,
            },
            "side": "sell",
        }
    });
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<ClientMessage>(check.clone()).expect("check quote parses")
        )
        .expect("round trips"),
        check
    );

    let received = json!({
        "type": "QuoteReceived",
        "data": {
            "rfq_id": Uuid::nil(),
            "terms": {"type": "call_spread", "lower_strike": 100, "upper_strike": 120},
            "side": "sell",
            "maker": "A87ysNWAmmqQHafLSQGbYSLhtcV3BjXctR88EWuSobGP",
            "price": 2,
            "valid_until": 3,
            "nonce": 4,
            "order_id": "01".repeat(32),
            "net_price": null,
        }
    });
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<ServerMessage>(received.clone()).expect("quote parses")
        )
        .expect("round trips"),
        received
    );
    for field in ["terms", "side", "net_price"] {
        let mut missing = received.clone();
        missing["data"]
            .as_object_mut()
            .expect("object")
            .remove(field);
        assert!(
            serde_json::from_value::<ServerMessage>(missing).is_err(),
            "{field} must be required"
        );
    }
}

#[test]
fn rfq_created_and_position_payouts_require_their_strict_keys() {
    let created = json!({
        "type": "RfqCreated",
        "data": {
            "rfq_id": Uuid::nil(),
            "rfq_version": 1,
            "terms": {"type": "covered_call", "strike": 100},
            "side": "sell",
            "expires_at": 2,
            "created_at": 1,
            "order_options": [],
            "request_id": null,
        }
    });
    for field in ["terms", "side"] {
        let mut missing = created.clone();
        missing["data"]
            .as_object_mut()
            .expect("object")
            .remove(field);
        assert!(
            serde_json::from_value::<ServerMessage>(missing).is_err(),
            "RfqCreated.{field} must be required"
        );
    }

    let positions = json!({
        "type": "Positions",
        "data": {
            "request_id": Uuid::nil(),
            "positions": [{
                "pda": "75NqraLLofiLUpNQg2wUPG6CJpxcaGEZUwuNrnAgvcTP",
                "market": "GQHjFXCxFNwgSYmU87k3eu5oCXCtD89WmS3DRzFX3SP9",
                "underlying_mint": "4WfbkL2bewdXcoxpqjjau4FynhwCt4KeEiSYKLESRfgy",
                "quote_mint": "7g3yp1HFGur2nvsU1PsqivfT3C21v1pFrKq99T5hLDzM",
                "terms": {"type": "call_spread", "lower_strike": 100, "upper_strike": 120},
                "side": "sell",
                "state": "open",
                "quantity": 1,
                "price": 2,
                "total_premium": 2,
                "created_at": 1,
                "expiry_ts": 2,
                "revision": {"slot": 100, "transaction_index": 0, "instruction_index": 1},
                "underlying_decimals": 9, "quote_decimals": 6,
            }],
        }
    });
    assert!(serde_json::from_value::<ServerMessage>(positions.clone()).is_ok());
    for field in [
        "revision",
        "state",
        "underlying_decimals",
        "quote_decimals",
        "total_premium",
    ] {
        let mut missing = positions.clone();
        missing["data"]["positions"][0]
            .as_object_mut()
            .expect("position")
            .remove(field);
        assert!(
            serde_json::from_value::<ServerMessage>(missing).is_err(),
            "Positions.{field} must be required"
        );
    }
}

#[test]
fn spread_liability_and_stats_fields_are_visible_and_required() {
    let liability = json!({
        "quote_mint": "7g3yp1HFGur2nvsU1PsqivfT3C21v1pFrKq99T5hLDzM",
        "symbol": "USDC",
        "current_liability": 10,
        "policy": { "type": "unconfigured" },
    });
    let decoded = serde_json::from_value::<QuoteLiabilityCapInfo>(liability.clone())
        .expect("unconfigured liability parses");
    assert_eq!(
        decoded.policy,
        acta_maker_sdk::types::QuoteLiabilityPolicy::Unconfigured {}
    );
    assert_eq!(
        serde_json::to_value(decoded).expect("round trips"),
        liability
    );

    let mut missing = liability.clone();
    missing.as_object_mut().expect("object").remove("policy");
    assert!(serde_json::from_value::<QuoteLiabilityCapInfo>(missing).is_err());

    let stats = json!({
        "usd": null,
        "total_volume_24h": 1,
        "total_trades_24h": 2,
        "spread_trades_24h": 3,
        "total_price_24h": 4,
        "active_markets": 5,
        "active_makers": 6,
        "active_rfqs": 7,
    });
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<GlobalStats>(stats.clone()).expect("stats parse")
        )
        .expect("round trips"),
        stats
    );
    let mut missing = stats;
    missing
        .as_object_mut()
        .expect("object")
        .remove("spread_trades_24h");
    assert!(serde_json::from_value::<GlobalStats>(missing).is_err());
}

#[test]
fn subscribe_roundtrip() {
    let msg = ClientMessage::Subscribe(SubscribeData {
        products: acta_maker_sdk::ProductSet::ALL,
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs, WsChannel::Trades],
        underlying_mints: vec![acta_maker_sdk::Address32::new([165; 32])],
        quote_mints: vec![],
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ClientMessage::Subscribe(data) => {
            assert_eq!(data.channels.len(), 2);
            assert_eq!(data.underlying_mints.len(), 1);
        }
        _ => panic!("expected Subscribe"),
    }
}

#[test]
fn request_error_roundtrip() {
    let original = ServerMessage::RequestError(RequestErrorEnvelope {
        request_id: Uuid::new_v4(),
        error: ServerError::RfqNotFound,
        delivery: None,
    });
    let json = serde_json::to_value(&original).unwrap();
    let parsed: ServerMessage = serde_json::from_value(json).unwrap();
    match parsed {
        ServerMessage::RequestError(env) => {
            assert!(matches!(env.error, ServerError::RfqNotFound));
        }
        _ => panic!("expected RequestError"),
    }
}

#[test]
fn subscribe_ack_roundtrip() {
    let original = ServerMessage::SubscribeAck(SubscribeAckData {
        request_id: Uuid::new_v4(),
        subscribed: vec![common::WsChannel::Rfqs],
    });
    let json = serde_json::to_value(&original).unwrap();
    let parsed: ServerMessage = serde_json::from_value(json).unwrap();
    match parsed {
        ServerMessage::SubscribeAck(data) => {
            assert_eq!(data.subscribed, vec![common::WsChannel::Rfqs]);
        }
        _ => panic!("expected SubscribeAck"),
    }
}

#[test]
fn unsubscribe_ack_roundtrip() {
    let original = ServerMessage::UnsubscribeAck(UnsubscribeAckData {
        request_id: Uuid::new_v4(),
        unsubscribed: vec![common::WsChannel::Markets],
    });
    let json = serde_json::to_value(&original).unwrap();
    let parsed: ServerMessage = serde_json::from_value(json).unwrap();
    match parsed {
        ServerMessage::UnsubscribeAck(data) => {
            assert_eq!(data.unsubscribed, vec![common::WsChannel::Markets]);
        }
        _ => panic!("expected UnsubscribeAck"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn quote_roundtrip_prop(
        rfq_bytes in proptest::array::uniform16(any::<u8>()),
        strike in any::<u64>(),
        price in any::<u64>(),
        valid_until in 0u64..2_000_000_000u64,
        nonce in any::<u64>(),
        order_bytes in proptest::array::uniform32(any::<u8>()),
    ) {
        let rfq_id = Uuid::from_bytes(rfq_bytes);
        let order_id = OrderId::new(order_bytes);
        let msg = ClientMessage::Quote(QuoteMessage {
            rfq_id,
            terms: OptionTerms::CoveredCall {
                strike: Strike::new(strike),
            },
            side: TakerSide::Sell,
            price: Price::new(price),
            valid_until: QuoteExpiry::from_unix_seconds(valid_until),
            nonce: Nonce::new(nonce),
            order_id,
            signature: "sig".to_string(),
        });

        let raw = serde_json::to_string(&msg).unwrap();
        let decoded: ClientMessage = serde_json::from_str(&raw).unwrap();
        match decoded {
            ClientMessage::Quote(decoded) => {
                prop_assert_eq!(decoded.rfq_id, rfq_id);
                prop_assert_eq!(decoded.terms.strike(), Strike::new(strike));
                prop_assert_eq!(decoded.price, Price::new(price));
                prop_assert_eq!(decoded.valid_until, QuoteExpiry::from_unix_seconds(valid_until));
                prop_assert_eq!(decoded.nonce, Nonce::new(nonce));
                prop_assert_eq!(decoded.order_id, order_id);
            }
            _ => prop_assert!(false, "decoded wrong variant"),
        }
    }

    #[test]
    fn rfq_closed_roundtrip_prop(
        rfq_bytes in proptest::array::uniform16(any::<u8>()),
        rfq_version in any::<u64>(),
        closed_at in 0u64..2_000_000_000u64,
        reason in prop_oneof![
            Just(RfqCloseReason::Expired),
            Just(RfqCloseReason::TakerCancelled),
            Just(RfqCloseReason::Filled),
            Just(RfqCloseReason::MarketExpired),
            Just(RfqCloseReason::LadderTimeout),
        ],
    ) {
        let rfq_id = Uuid::from_bytes(rfq_bytes);
        let msg = ServerMessage::RfqClosed(RfqClosedMessage {
            rfq_id,
            rfq_version: RfqVersion::new(rfq_version),
            reason,
            your_quote: None,
            winner: None,
            closed_at: UNIX_EPOCH + Duration::from_secs(closed_at),
        });

        let raw = serde_json::to_string(&msg).unwrap();
        let decoded: ServerMessage = serde_json::from_str(&raw).unwrap();
        match decoded {
            ServerMessage::RfqClosed(decoded) => {
                prop_assert_eq!(decoded.rfq_id, rfq_id);
                prop_assert_eq!(decoded.rfq_version, RfqVersion::new(rfq_version));
                prop_assert_eq!(decoded.reason, reason);
                prop_assert_eq!(decoded.closed_at, UNIX_EPOCH + Duration::from_secs(closed_at));
            }
            _ => prop_assert!(false, "decoded wrong variant"),
        }
    }

    #[test]
    fn quote_cancelled_roundtrip_prop(
        rfq_bytes in proptest::array::uniform16(any::<u8>()),
        cancelled_at in 0u64..2_000_000_000u64,
        reason in prop_oneof![
            Just(QuoteCancelReason::Requested),
            Just(QuoteCancelReason::RiskCheck),
            Just(QuoteCancelReason::RfqAccepted),
        ],
    ) {
        let rfq_id = Uuid::from_bytes(rfq_bytes);
        let msg = ServerMessage::QuoteCancelled(QuoteCancelledMessage {
            rfq_id,
            order_ids: Vec::new(),
            reason,
            cancelled_at: UNIX_EPOCH + Duration::from_secs(cancelled_at),
        });

        let raw = serde_json::to_string(&msg).unwrap();
        let decoded: ServerMessage = serde_json::from_str(&raw).unwrap();
        match decoded {
            ServerMessage::QuoteCancelled(decoded) => {
                prop_assert_eq!(decoded.rfq_id, rfq_id);
                prop_assert_eq!(decoded.reason, reason);
                prop_assert_eq!(decoded.cancelled_at, UNIX_EPOCH + Duration::from_secs(cancelled_at));
            }
            _ => prop_assert!(false, "decoded wrong variant"),
        }
    }
}

#[test]
fn quote_rejected_roundtrip() {
    let rfq_id = Uuid::new_v4();
    let msg = ServerMessage::QuoteRejected(QuoteRejectedMessage {
        rfq_id,
        order_id: OrderId([0xbb; 32]),
        reason: QuoteRejectReason::InvalidStrike,
        message: "strike not in allowed set".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"QuoteRejected\""));
    assert!(json.contains("\"invalid_strike\""));
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::QuoteRejected(data) => {
            assert_eq!(data.rfq_id, rfq_id);
            assert_eq!(data.reason, QuoteRejectReason::InvalidStrike);
        }
        _ => panic!("Expected QuoteRejected"),
    }
}

#[test]
fn cancel_all_quotes_ack_roundtrip() {
    let request_id = Uuid::new_v4();
    let msg = ServerMessage::CancelAllQuotesAck(CancelAllQuotesAckMessage {
        request_id,
        cancelled_count: 3,
        cancelled_order_ids: vec![
            OrderId([0xaa; 32]),
            OrderId([0xbb; 32]),
            OrderId([0xcc; 32]),
        ],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"CancelAllQuotesAck\""));
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::CancelAllQuotesAck(data) => {
            assert_eq!(data.request_id, request_id);
            assert_eq!(data.cancelled_count, 3);
            assert_eq!(data.cancelled_order_ids.len(), 3);
        }
        _ => panic!("Expected CancelAllQuotesAck"),
    }
}

#[test]
fn replace_quote_roundtrip() {
    let rfq_id = Uuid::new_v4();
    let msg = ClientMessage::ReplaceQuote(ReplaceQuoteMessage {
        old_order_id: OrderId([0xaa; 32]),
        rfq_id,
        terms: OptionTerms::CoveredCall {
            strike: Strike::new(100_000_000_000),
        },
        side: TakerSide::Sell,
        price: Price::new(5_000_000),
        valid_until: QuoteExpiry::from_unix_seconds(1_700_000_000),
        nonce: Nonce::new(42),
        order_id: OrderId([0xbb; 32]),
        signature: "test_sig".to_string(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"ReplaceQuote\""));
    assert!(json.contains("\"old_order_id\""));
    let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ClientMessage::ReplaceQuote(data) => {
            assert_eq!(data.rfq_id, rfq_id);
            assert_eq!(data.old_order_id, OrderId([0xaa; 32]));
        }
        _ => panic!("Expected ReplaceQuote"),
    }
}

#[test]
fn batch_quotes_roundtrip() {
    let q1 = QuoteMessage {
        rfq_id: Uuid::new_v4(),
        terms: OptionTerms::CoveredCall {
            strike: Strike::new(100_000_000_000),
        },
        side: TakerSide::Sell,
        price: Price::new(5_000_000),
        valid_until: QuoteExpiry::from_unix_seconds(1_700_000_000),
        nonce: Nonce::new(1),
        order_id: OrderId([0xaa; 32]),
        signature: "sig1".to_string(),
    };
    let q2 = QuoteMessage {
        rfq_id: Uuid::new_v4(),
        terms: OptionTerms::CoveredCall {
            strike: Strike::new(110_000_000_000),
        },
        side: TakerSide::Sell,
        price: Price::new(3_000_000),
        valid_until: QuoteExpiry::from_unix_seconds(1_700_000_000),
        nonce: Nonce::new(2),
        order_id: OrderId([0xbb; 32]),
        signature: "sig2".to_string(),
    };
    let msg = ClientMessage::BatchQuotes(BatchQuotesMessage {
        quotes: vec![q1, q2],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"BatchQuotes\""));
    let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ClientMessage::BatchQuotes(data) => assert_eq!(data.quotes.len(), 2),
        _ => panic!("Expected BatchQuotes"),
    }
}

#[test]
fn batch_quotes_ack_roundtrip() {
    let ack = QuoteAcknowledgedMessage {
        rfq_id: Uuid::new_v4(),
        order_id: OrderId([0xaa; 32]),
        replaced_order_id: None,
    };
    let reject = QuoteRejectedMessage {
        rfq_id: Uuid::new_v4(),
        order_id: OrderId([0xbb; 32]),
        reason: QuoteRejectReason::RfqNotActive,
        message: "rejected".to_string(),
    };
    let msg = ServerMessage::BatchQuotesAck(BatchQuotesAckMessage {
        results: vec![
            BatchQuoteResult::Acknowledged(ack),
            BatchQuoteResult::Rejected(reject),
        ],
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"BatchQuotesAck\""));
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::BatchQuotesAck(data) => {
            assert_eq!(data.results.len(), 2);
            assert!(matches!(data.results[0], BatchQuoteResult::Acknowledged(_)));
            assert!(matches!(data.results[1], BatchQuoteResult::Rejected(_)));
        }
        _ => panic!("Expected BatchQuotesAck"),
    }
}

#[test]
fn unknown_types_and_codes_are_rejected() {
    for raw in [
        r#"{"type":"SomeFutureMessage","data":{"x":1}}"#,
        r#"{"type":"Error","data":{"type":"Generic","data":{"code":"brand_new_code","message":"x"}}}"#,
        r#"{"type":"Error","data":{"type":"Cap","data":{"future_cap":{"limit":1}}}}"#,
        r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":"quota_exhausted","message":"x"}}"#,
        r#"{"type":"BatchQuotesAck","data":{"results":[{"status":"deferred","data":{}}]}}"#,
    ] {
        assert!(parse_server_message(raw).is_err(), "{raw}");
    }
}

#[test]
fn chain_event_identity_requires_instruction_index() {
    assert!(parse_server_message(
        r#"{"type":"ChainEvent","data":{"event_type":"PositionSettled","signature":"sig","slot":1,"position":"75NqraLLofiLUpNQg2wUPG6CJpxcaGEZUwuNrnAgvcTP"}}"#,
    ).is_err());
    let identified = parse_server_message(
        r#"{"type":"ChainEvent","data":{"event_type":"PositionSettled","signature":"sig","instruction_index":3,"slot":1,"position":"75NqraLLofiLUpNQg2wUPG6CJpxcaGEZUwuNrnAgvcTP"}}"#,
    )
    .unwrap();

    let ServerMessage::ChainEvent(identified) = identified else {
        panic!("expected identified chain event");
    };
    let identity = identified.identity();
    assert_eq!(identity.signature, "sig");
    assert_eq!(identity.instruction_index, 3);
}

#[test]
fn referral_protocol_messages_match_backend_wire_shape() {
    let request_id = Uuid::new_v4();
    let client = ClientMessage::RedeemInvite(RedeemInviteData {
        request_id,
        code: "ABCD".to_string(),
    });
    assert_eq!(client.request_id(), Some(request_id));

    let raw = format!(
        r#"{{"type":"InviteRedeemed","data":{{"request_id":"{request_id}","referral_code":"abcd"}}}}"#
    );
    match parse_server_message(&raw).unwrap() {
        ServerMessage::InviteRedeemed(data) => {
            assert_eq!(data.request_id, request_id);
            assert_eq!(data.referral_code.as_str(), "ABCD");
        }
        _ => panic!("Expected InviteRedeemed"),
    }
}

#[test]
fn my_quotes_default_requests_complete_live_quotes() {
    let request = GetMyQuotesMessage::default();
    assert_eq!(request.scope, MakerQuoteScope::Live {});
}

#[test]
fn my_quotes_requires_has_more_on_the_wire() {
    let parsed = serde_json::from_value::<ServerMessage>(json!({
        "type": "MyQuotes",
        "data": {
            "request_id": Uuid::new_v4(),
            "quotes": []
        }
    }));
    assert!(parsed.is_err(), "has_more is required on the wire");
}

#[test]
fn default_request_messages_receive_unique_non_nil_ids() {
    let first = GetMarketsMessage::default().request_id;
    let second = GetMarketsMessage::default().request_id;

    assert!(!first.is_nil());
    assert!(!second.is_nil());
    assert_ne!(first, second);
}

#[test]
fn indicative_price_response_is_not_treated_as_an_acknowledged_request() {
    let message = ClientMessage::IndicativePricesResponse(IndicativePricesResponseMessage {
        request_id: Uuid::new_v4(),
        market: MarketId::new(acta_maker_sdk::Address32::new([
            228, 214, 147, 210, 78, 100, 108, 183, 135, 132, 38, 34, 93, 144, 210, 96, 127, 212,
            14, 46, 124, 130, 244, 207, 130, 70, 178, 204, 62, 13, 236, 184,
        ])),
        position_type: PositionType::CoveredCall,
        prices: vec![],
    });
    assert_eq!(message.request_id(), None);
}

#[test]
fn known_message_with_bad_payload_is_still_an_error() {
    assert!(parse_server_message(r#"{"type":"RfqCreated","data":{"x":1}}"#).is_err());
}

#[test]
fn wrong_endpoint_error_parses() {
    let parsed: ServerMessage = serde_json::from_str(
        r#"{"type":"Error","data":{"type":"WrongEndpoint","data":{"endpoint":"maker_data","allowed_endpoints":["maker"]}}}"#,
    )
    .unwrap();
    match parsed {
        ServerMessage::Error(ServerError::WrongEndpoint {
            endpoint,
            allowed_endpoints,
        }) => {
            assert_eq!(endpoint, WsEndpointKind::MakerData);
            assert_eq!(allowed_endpoints, vec![WsEndpointKind::Maker]);
        }
        _ => panic!("Expected WrongEndpoint"),
    }
}

#[test]
fn maker_positions_reports_rows_the_server_cannot_describe() {
    let request_id = Uuid::new_v4();
    let raw = json!({
        "type": "MakerPositions",
        "data": {
            "request_id": request_id,
            "positions": [],
            "unrenderable": [{
                "pda": "HYe9CryCnQ3ZviCwoG8X7REFoSBK7HCjrhWWMpNtTah1",
                "market": "G5ca7M9SYp8uTzN6rGe1BWrZAtqnNqb1FyBVTnjVNNGs",
                "created_at": 1_710_000_000,
                "missing": ["underlying_symbol", "quote_decimals"]
            }],
            "has_more": true
        }
    });

    let parsed: ServerMessage = serde_json::from_value(raw).unwrap();
    let ServerMessage::MakerPositions(data) = parsed else {
        panic!("expected MakerPositions");
    };
    assert_eq!(data.request_id, request_id);
    assert_eq!(data.unrenderable.len(), 1);
    assert_eq!(
        data.unrenderable[0].pda,
        acta_maker_sdk::Address32::from_base58("HYe9CryCnQ3ZviCwoG8X7REFoSBK7HCjrhWWMpNtTah1")
            .unwrap()
    );
    assert_eq!(
        data.unrenderable[0].market,
        MarketId::new(acta_maker_sdk::Address32::new([
            224, 13, 216, 227, 148, 24, 8, 96, 220, 157, 76, 69, 181, 212, 22, 40, 163, 23, 3, 127,
            86, 55, 215, 95, 9, 41, 3, 226, 42, 178, 145, 20
        ]))
    );
    assert_eq!(
        data.unrenderable[0].created_at,
        UNIX_EPOCH + Duration::from_secs(1_710_000_000)
    );
    assert_eq!(
        data.unrenderable[0].missing,
        ["underlying_symbol", "quote_decimals"]
    );
    assert!(data.has_more);
}

#[test]
fn maker_positions_require_the_unrenderable_list() {
    let raw = json!({
        "type": "MakerPositions",
        "data": {
            "request_id": Uuid::new_v4(),
            "positions": [],
            "has_more": false
        }
    });
    assert!(serde_json::from_value::<ServerMessage>(raw).is_err());
}

#[test]
fn listing_cursors_are_tagged_keyset_positions() {
    let request = GetMakerPositionsMessage {
        request_id: Uuid::new_v4(),
        market: acta_maker_sdk::ws::types::QueryFilter::All {},
        underlying_mint: acta_maker_sdk::ws::types::QueryFilter::All {},
        status: Vec::new(),
        min_expiry_ts: acta_maker_sdk::ws::types::TimeBound::Unbounded {},
        limit: 100,
        cursor: acta_maker_sdk::ws::types::PageCursor::After {
            at: UNIX_EPOCH + Duration::from_secs(1_800_000_000),
            id: acta_maker_sdk::Address32::new([5; 32]),
        },
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["cursor"]["type"], "after");
    assert_eq!(json["cursor"]["at"], 1_800_000_000);

    let order_id = OrderId([0xab; 32]);
    let quotes = GetMyQuotesMessage {
        request_id: Uuid::new_v4(),
        scope: MakerQuoteScope::History {
            limit: 100,
            cursor: acta_maker_sdk::ws::types::PageCursor::After {
                at: UNIX_EPOCH + Duration::from_secs(1_800_000_777),
                id: order_id,
            },
        },
    };
    let json = serde_json::to_value(&quotes).unwrap();
    assert_eq!(json["scope"]["cursor"]["at"], 1_800_000_777);
    let back: GetMyQuotesMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back.scope, quotes.scope);

    let missing_scope = serde_json::from_value::<GetMyQuotesMessage>(json!({
        "request_id": Uuid::new_v4(),
        "limit": 5
    }));
    assert!(missing_scope.is_err(), "scope is required on the wire");
}

#[test]
fn quote_rejected_cap_exceeded_carries_the_cap_error() {
    let raw = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":{"cap_exceeded":"caps_unavailable"},"message":"rejected"}}"#;
    match serde_json::from_str::<ServerMessage>(raw).unwrap() {
        ServerMessage::QuoteRejected(data) => {
            assert_eq!(
                data.reason,
                QuoteRejectReason::CapExceeded(CapError::CapsUnavailable)
            );
        }
        _ => panic!("Expected QuoteRejected"),
    }

    let structured = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":{"cap_exceeded":{"maker_position_cap_exceeded":{"current":3,"limit":3}}},"message":"rejected"}}"#;
    match serde_json::from_str::<ServerMessage>(structured).unwrap() {
        ServerMessage::QuoteRejected(data) => assert!(matches!(
            data.reason,
            QuoteRejectReason::CapExceeded(CapError::MakerPositionCapExceeded {
                current: 3,
                limit: 3
            })
        )),
        _ => panic!("Expected QuoteRejected"),
    }

    let plain = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":"invalid_strike","message":"rejected"}}"#;
    match serde_json::from_str::<ServerMessage>(plain).unwrap() {
        ServerMessage::QuoteRejected(data) => {
            assert_eq!(data.reason, QuoteRejectReason::InvalidStrike)
        }
        _ => panic!("Expected QuoteRejected"),
    }
}

#[test]
fn spread_liability_caps_decode() {
    for (code, expected) in [
        (
            "quote_liability_cap_exceeded",
            "quote_liability_cap_exceeded",
        ),
        (
            "maker_quote_liability_cap_exceeded",
            "maker_quote_liability_cap_exceeded",
        ),
    ] {
        let raw = format!(
            r#"{{"type":"RfqSkipped","data":{{"rfq_id":"00000000-0000-0000-0000-00000000002a","market_id":"Ebjh5RZR799aVdZR9yePMNmEVmGibvjM3Zqr4kEegSbS","quantity":5,"cap_detail":{{"{code}":{{"quote_mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","current":7,"limit":9}}}}}}}}"#
        );
        match serde_json::from_str::<ServerMessage>(&raw).unwrap() {
            ServerMessage::RfqSkipped(data) => assert_eq!(data.cap_detail.code(), expected),
            _ => panic!("Expected RfqSkipped"),
        }
    }
}

#[test]
fn rfq_skipped_requires_cap_detail() {
    let raw = r#"{"type":"RfqSkipped","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","market_id":"Ebjh5RZR799aVdZR9yePMNmEVmGibvjM3Zqr4kEegSbS","quantity":5,"cap_detail":{"maker_position_cap_exceeded":{"current":10,"limit":10}}}}"#;
    match serde_json::from_str::<ServerMessage>(raw).unwrap() {
        ServerMessage::RfqSkipped(data) => {
            assert!(matches!(
                data.cap_detail,
                CapError::MakerPositionCapExceeded {
                    current: 10,
                    limit: 10
                }
            ));
        }
        _ => panic!("Expected RfqSkipped"),
    }

    let legacy = r#"{"type":"RfqSkipped","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","market_id":"Ebjh5RZR799aVdZR9yePMNmEVmGibvjM3Zqr4kEegSbS","quantity":5,"reason":"other"}}"#;
    assert!(serde_json::from_str::<ServerMessage>(legacy).is_err());
}

#[test]
fn cancel_quote_ack_requires_all_fields() {
    let ack = serde_json::json!({
        "type": "CancelQuoteAck",
        "data": {
            "request_id": "00000000-0000-0000-0000-000000000001",
            "rfq_id": "00000000-0000-0000-0000-000000000002",
            "cancelled_order_ids": []
        }
    });
    let decoded: acta_maker_sdk::ws::types::ServerMessage =
        serde_json::from_value(ack.clone()).unwrap();
    assert!(matches!(
        decoded,
        acta_maker_sdk::ws::types::ServerMessage::CancelQuoteAck(_)
    ));
    for field in ["request_id", "rfq_id", "cancelled_order_ids"] {
        let mut missing = ack.clone();
        missing["data"].as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<acta_maker_sdk::ws::types::ServerMessage>(missing).is_err(),
            "{field}"
        );
    }
}

#[test]
fn lifecycle_identity_fields_are_required() {
    let rfq_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let order_id = "01".repeat(32);
    let cases = [
        (
            "RfqCreated",
            "rfq_version",
            json!({
                "type": "RfqCreated",
                "data": {
                    "rfq_id": rfq_id,
                    "rfq_version": 1,
                    "terms": {"type": "covered_call", "strike": 1},
                    "side": "sell",
                    "expires_at": 1,
                    "created_at": 1,
                    "request_id": null,
                    "order_options": [],
                },
            }),
        ),
        (
            "RfqClosed",
            "rfq_version",
            json!({
                "type": "RfqClosed",
                "data": {
                    "rfq_id": rfq_id,
                    "rfq_version": 1,
                    "reason": "expired",
                    "closed_at": 1,
                    "your_quote": null,
                    "winner": null,
                },
            }),
        ),
        (
            "RfqAvailableAgain",
            "rfq_version",
            json!({
                "type": "RfqAvailableAgain",
                "data": {
                    "rfq_id": rfq_id,
                    "rfq_version": 1,
                    "reason": "signature_timeout",
                    "available_again_at": 1,
                },
            }),
        ),
        (
            "QuoteCancelled",
            "order_ids",
            json!({
                "type": "QuoteCancelled",
                "data": {
                    "rfq_id": rfq_id,
                    "order_ids": [],
                    "reason": "requested",
                    "cancelled_at": 1,
                },
            }),
        ),
        (
            "OrderAccepted",
            "order_version",
            json!({
                "type": "OrderAccepted",
                "data": { "order_id": order_id.as_str(), "order_version": 1 },
            }),
        ),
        (
            "OrderSubmitted",
            "order_version",
            json!({
                "type": "OrderSubmitted",
                "data": {
                    "order_id": order_id.as_str(),
                    "tx_signature": "signature",
                    "order_version": 2,
                },
            }),
        ),
        (
            "OrderConfirmed",
            "order_version",
            json!({
                "type": "OrderConfirmed",
                "data": {
                    "order_id": order_id.as_str(),
                    "position_pda": "75NqraLLofiLUpNQg2wUPG6CJpxcaGEZUwuNrnAgvcTP",
                    "order_version": 4,
                },
            }),
        ),
        (
            "OrderFailed",
            "order_version",
            json!({
                "type": "OrderFailed",
                "data": {
                    "order_id": order_id.as_str(),
                    "reason": "failed",
                    "order_version": 3,
                },
            }),
        ),
        (
            "OrderStatus",
            "state",
            json!({
                "type": "OrderStatus",
                "data": {
                    "request_id": request_id,
                    "order_id": order_id.as_str(),
                    "state": { "type": "pending" },
                },
            }),
        ),
    ];

    for (message_type, field, mut payload) in cases {
        let valid = serde_json::to_string(&payload).expect("serialize valid lifecycle fixture");
        let _valid = parse_server_message(&valid)
            .unwrap_or_else(|error| panic!("valid {message_type} fixture: {error}"));
        let removed = payload["data"]
            .as_object_mut()
            .expect("message data object")
            .remove(field);
        assert!(removed.is_some(), "{message_type} fixture lacks {field}");
        let malformed =
            serde_json::to_string(&payload).expect("serialize malformed lifecycle fixture");
        let error = parse_server_message(&malformed)
            .expect_err("missing lifecycle identity field must fail");
        assert!(
            error.to_string().contains(field),
            "{message_type} missing {field}: {error}"
        );
    }
}

#[test]
fn required_wire_fields_preserve_values_and_reject_missing_or_null() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/required_fields.json")).unwrap();
    assert_eq!(cases.len(), 11);
    for case in cases {
        let field = case["field"].as_str().unwrap();
        let message = &case["message"];
        let parsed: ServerMessage = serde_json::from_value(message.clone()).unwrap();
        let encoded = serde_json::to_value(parsed).unwrap();
        assert_eq!(encoded["data"][field], message["data"][field]);
        for null in [false, true] {
            let mut incomplete = message.clone();
            if null {
                incomplete["data"][field] = serde_json::Value::Null;
            } else {
                incomplete["data"].as_object_mut().unwrap().remove(field);
            }
            assert!(
                serde_json::from_value::<ServerMessage>(incomplete).is_err(),
                "{} requires {field}",
                message["type"]
            );
        }
    }
}

#[test]
fn reference_frames_decode() {
    let tokens: ServerMessage = serde_json::from_value(json!({
        "type": "Tokens",
        "data": {
            "request_id": "00000000-0000-0000-0000-000000000000",
            "tokens": [{ "mint": "bvgRxX4i3TVxu72eSGE1dkLjPMunReapc72mEmn3E6U", "symbol": "SOL", "decimals": 9, "oracle_feed_ids": [], "role": { "type": "underlying", "size_rule": { "min_size": 1, "max_size": 10, "step": 1 }, "strike_tick": 500000000, "quote_mints": ["63QRh6CpNaGpV13A7eV99sdKhBE2z1Yh8knawNfMqqbm"] } }]
        }
    }))
    .unwrap();
    let ServerMessage::Tokens(tokens) = tokens else {
        panic!("expected Tokens");
    };
    let TokenRole::Underlying { quote_mints, .. } = &tokens.tokens[0].role else {
        panic!("expected an underlying token");
    };
    assert_eq!(
        quote_mints,
        &[
            acta_maker_sdk::Address32::from_base58("63QRh6CpNaGpV13A7eV99sdKhBE2z1Yh8knawNfMqqbm")
                .expect("fixture")
        ]
    );

    let changed: ServerMessage = serde_json::from_value(json!({
        "type": "ReferenceChanged",
        "data": { "tokens_etag": "\"t\"", "markets_etag": "\"m\"" }
    }))
    .unwrap();
    assert!(matches!(
        changed,
        ServerMessage::ReferenceChanged(data) if data.markets_etag == "\"m\""
    ));
}

#[test]
fn settlement_variants_require_exact_fields() {
    use acta_maker_sdk::ws::types::PositionSettlement;
    for value in [
        json!({"type":"physical","settlement_price":100,"is_otm":true}),
        json!({"type":"cash","settlement_price":100,"maker_payoff":0,"taker_return":12}),
    ] {
        let decoded: PositionSettlement = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
    for value in [
        json!({"type":"open","maker_payoff":null}),
        json!({"type":"physical","settlement_price":100}),
        json!({"type":"cash","settlement_price":100,"maker_payoff":0}),
        json!({"type":"cash","settlement_price":100,"maker_payoff":null,"taker_return":1}),
        json!({"type":"unknown"}),
    ] {
        assert!(serde_json::from_value::<PositionSettlement>(value).is_err());
    }
}

#[test]
fn hello_requires_disconnect_policy_and_client_identity() {
    let good = serde_json::json!({
        "protocol_version": acta_maker_sdk::WS_PROTOCOL_VERSION,
        "on_disconnect": "keep_quotes",
        "client_name": "bot",
        "client_version": "1.0.0",
    });
    assert!(serde_json::from_value::<acta_maker_sdk::ws::types::HelloData>(good.clone()).is_ok());
    for field in ["on_disconnect", "client_name", "client_version"] {
        let mut missing = good.clone();
        let _ = missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<acta_maker_sdk::ws::types::HelloData>(missing).is_err());
    }
    let mut extra = good;
    extra["features"] = serde_json::json!([]);
    assert!(serde_json::from_value::<acta_maker_sdk::ws::types::HelloData>(extra).is_err());
}

#[test]
fn position_state_has_one_required_tag_and_no_open_settlement() {
    use acta_maker_sdk::ws::types::PositionState;
    for value in [
        json!({"state":"open"}),
        json!({"state":"funded"}),
        json!({"state":"liquidated"}),
        json!({"state":"settled","settlement":{"type":"physical","settlement_price":100,"is_otm":true}}),
        json!({"state":"settled","settlement":{"type":"cash","settlement_price":100,"maker_payoff":0,"taker_return":12}}),
    ] {
        let decoded: PositionState = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
    for value in [
        json!({"status":"open"}),
        json!({"state":"settled"}),
        json!({"state":"open","settlement":{"type":"cash","settlement_price":1,"maker_payoff":0,"taker_return":1}}),
    ] {
        assert!(serde_json::from_value::<PositionState>(value).is_err());
    }
}
