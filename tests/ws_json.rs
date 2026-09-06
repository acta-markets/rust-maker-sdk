use acta_maker_sdk::{
    Decimals, MarketId, Nonce, OrderId, OrderVersion, PositionType, Price, QuoteExpiry, RfqVersion,
    Strike,
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
        json!({"session_id":"s","expires_at":null}),
    ] {
        assert!(
            serde_json::from_value::<ServerMessage>(json!({"type":"AuthSuccess","data":data}))
                .is_err()
        );
    }
    let message: ServerMessage = serde_json::from_value(json!({
        "type":"AuthSuccess", "data":{"session_id":"s","expires_at":1_710_086_400}
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
            "position_pda": "position",
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
fn auth_error_parses_optional_message() {
    let raw_with_message = json!({
        "type": "AuthError",
        "data": {
            "reason": "invalid_signature",
            "message": "bad signature bytes"
        }
    });
    let parsed_with_message: ServerMessage = serde_json::from_value(raw_with_message).unwrap();
    match parsed_with_message {
        ServerMessage::AuthError(data) => {
            assert_eq!(data.reason, "invalid_signature");
            assert!(!data.is_session_expired());
            assert_eq!(data.message.as_deref(), Some("bad signature bytes"));
        }
        _ => panic!("expected AuthError"),
    }

    let raw_without_message = json!({
        "type": "AuthError",
        "data": {
            "reason": "session_expired"
        }
    });
    let parsed_without_message: ServerMessage =
        serde_json::from_value(raw_without_message).unwrap();
    match parsed_without_message {
        ServerMessage::AuthError(data) => {
            assert_eq!(data.reason, "session_expired");
            assert!(data.is_session_expired());
            assert_eq!(data.message, None);
        }
        _ => panic!("expected AuthError"),
    }
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
                        "strike": 1,
                        "maker": "maker",
                        "price": 2,
                        "valid_until": 3,
                        "nonce": 4,
                        "order_id": "0000000000000000000000000000000000000000000000000000000000000002"
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
                    "owner": "owner",
                    "maker_pda": "pda",
                    "quote_signing": "legacy-key"
                }
            }),
            ExpectedParse::ChainEventMakerRegistered {
                quote_signing: "legacy-key",
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
                assert_eq!(event.quote_signing, expected, "{name}");
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
                        "program_id": "Program1111111111111111111111111111111111111",
                        "market_pda": "Market11111111111111111111111111111111111111",
                        "underlying_mint": "Underlying11111111111111111111111111111111111",
                        "quote_mint": "Quote111111111111111111111111111111111111111",
                        "expiry_ts": 1_700_000_000,
                        "is_put": false,
                        "collateral_mint": "Collateral1111111111111111111111111111111111",
                        "settlement_mint": "Settlement111111111111111111111111111111111"
                    },
                    "underlying_oracle_pda": "UnderlyingOracle1111111111111111111111111111",
                    "quote_oracle_pda": "QuoteOracle11111111111111111111111111111111",
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
                "UnderlyingOracle1111111111111111111111111111"
            );
            assert_eq!(
                market.quote_oracle_pda,
                "QuoteOracle11111111111111111111111111111111"
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
        market: acta_maker_sdk::MarketId::new("market"),
        position_type: PositionType::CoveredCall,
        updated_at: UNIX_EPOCH,
        is_stale: false,
        strikes: vec![],
    });
    let json = serde_json::to_string(&msg).expect("serialize indicative prices");
    assert!(json.contains("\"request_id\""));

    let missing_request_id = r#"{"type":"IndicativePrices","data":{"market":"market","position_type":"covered_call","updated_at":0,"is_stale":false,"strikes":[]}}"#;
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
        enabled_features: vec!["quote_expired".to_string()],
        server_time_unix_ms: UNIX_EPOCH + Duration::from_millis(1_700_000_000_000),
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
fn rfq_broadcast_roundtrip() {
    use acta_maker_sdk::{PositionType, Quantity};

    let rfq_id = Uuid::new_v4();
    let msg = ServerMessage::RfqBroadcast(RfqBroadcastMessage {
        rfq_id,
        market: MarketDescriptor {
            chain_id: acta_maker_sdk::ChainId::new(0),
            program_id: "prog".to_string(),
            market_pda: "market".to_string(),
            underlying_mint: "underlying".to_string(),
            quote_mint: "quote".to_string(),
            expiry_ts: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            is_put: false,
            collateral_mint: "collateral".to_string(),
            settlement_mint: "settlement".to_string(),
        },
        position_type: PositionType::CoveredCall,
        strike: Strike::new(100),
        quantity: Quantity::new(10),
        expires_at: UNIX_EPOCH + Duration::from_secs(1_700_000_100),
        taker: "taker_pubkey".to_string(),
        sent_at_unix_ms: UNIX_EPOCH + Duration::from_millis(1_700_000_099_250),
        order_options: vec![RfqOrderOption {
            strike: Strike::new(100),
        }],
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::RfqBroadcast(data) => {
            assert_eq!(data.rfq_id, rfq_id);
            assert_eq!(data.strike, Strike::new(100));
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
        "market":{"chain_id":0,"program_id":"p","market_pda":"m","underlying_mint":"u",
                  "quote_mint":"q","expiry_ts":1700000000,"is_put":false,
                  "collateral_mint":"c","settlement_mint":"s"},
        "position_type":"covered_call","strike":100,"quantity":10,
        "expires_at":1700000100,"taker":"t","order_options":[]}}"#;
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
            code: "test".to_string(),
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
        strike: Strike::new(50),
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
            assert_eq!(data.strike, Strike::new(50));
            assert_eq!(data.price, Price::new(100));
            assert_eq!(data.nonce, Nonce::new(42));
            assert_eq!(data.order_id, order_id);
            assert_eq!(data.signature, "base58sig");
        }
        _ => panic!("expected Quote"),
    }
}

#[test]
fn subscribe_roundtrip() {
    let msg = ClientMessage::Subscribe(SubscribeData {
        request_id: Uuid::new_v4(),
        channels: vec![WsChannel::Rfqs, WsChannel::Trades],
        underlying_mints: Some(vec!["mint1".to_string()]),
        quote_mints: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ClientMessage::Subscribe(data) => {
            assert_eq!(data.channels.len(), 2);
            assert_eq!(data.underlying_mints.unwrap().len(), 1);
        }
        _ => panic!("expected Subscribe"),
    }
}

#[test]
fn request_error_roundtrip() {
    let original = ServerMessage::RequestError(RequestErrorEnvelope {
        request_id: Uuid::new_v4(),
        error: ServerError::RfqNotFound,
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
            strike: Strike::new(strike),
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
                prop_assert_eq!(decoded.strike, Strike::new(strike));
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
        message: Some("strike not in allowed set".to_string()),
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
fn quote_rejected_without_message_field() {
    let msg = ServerMessage::QuoteRejected(QuoteRejectedMessage {
        rfq_id: Uuid::new_v4(),
        order_id: OrderId([0xcc; 32]),
        reason: QuoteRejectReason::CapExceeded(CapError::CapsUnavailable),
        message: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("\"message\""));
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
        strike: Strike::new(100_000_000_000),
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
        strike: Strike::new(100_000_000_000),
        price: Price::new(5_000_000),
        valid_until: QuoteExpiry::from_unix_seconds(1_700_000_000),
        nonce: Nonce::new(1),
        order_id: OrderId([0xaa; 32]),
        signature: "sig1".to_string(),
    };
    let q2 = QuoteMessage {
        rfq_id: Uuid::new_v4(),
        strike: Strike::new(110_000_000_000),
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
        message: None,
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
fn unknown_server_message_parses_to_unknown() {
    let raw = r#"{"type":"SomeFutureMessage","data":{"x":1}}"#;
    let parsed = parse_server_message(raw).unwrap();
    match parsed {
        ServerMessage::Unknown(unknown) => {
            assert_eq!(unknown.message_type, "SomeFutureMessage");
            assert_eq!(&*unknown.raw_json, raw);
        }
        _ => panic!("Expected Unknown"),
    }
}

#[test]
fn unknown_server_error_parses_to_unknown() {
    let parsed = parse_server_message(
        r#"{"type":"Error","data":{"type":"SomeFutureError","data":{"x":1}}}"#,
    )
    .unwrap();
    match parsed {
        ServerMessage::Error(ServerError::Unknown(unknown)) => {
            assert_eq!(unknown.error_type, "SomeFutureError");
            assert!(unknown.raw_json.contains("\"x\":1"));
        }
        _ => panic!("Expected Error"),
    }
}

#[test]
fn unknown_nested_quote_reason_does_not_drop_the_connection() {
    let parsed = parse_server_message(
        r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-000000000001","order_id":"0000000000000000000000000000000000000000000000000000000000000002","reason":"future_rejection"}}"#,
    )
    .unwrap();

    assert!(matches!(
        parsed,
        ServerMessage::QuoteRejected(QuoteRejectedMessage {
            reason: QuoteRejectReason::Unknown,
            ..
        })
    ));
}

#[test]
fn unknown_nested_batch_result_is_preserved() {
    let order_id = OrderId::new([9_u8; 32]);
    let raw = format!(
        r#"{{"type":"BatchQuotesAck","data":{{"results":[{{"status":"deferred","data":{{"order_id":"{order_id}","retry_after_ms":5}}}}]}}}}"#
    );
    let parsed = parse_server_message(&raw).unwrap();

    let ServerMessage::BatchQuotesAck(BatchQuotesAckMessage { results }) = parsed else {
        panic!("expected batch acknowledgement");
    };
    let [BatchQuoteResult::Unknown(unknown)] = results.as_slice() else {
        panic!("expected unknown batch result");
    };
    assert_eq!(unknown.status, "deferred");
    assert_eq!(results[0].order_id(), Some(order_id));
    assert_eq!(
        unknown.data.as_ref().unwrap()["retry_after_ms"],
        serde_json::json!(5)
    );
    let encoded = serde_json::to_string(&BatchQuoteResult::Unknown(unknown.clone())).unwrap();
    assert!(encoded.contains(r#""status":"deferred""#));
    assert!(encoded.contains(r#""retry_after_ms":5"#));
}

#[test]
fn unknown_nested_chain_event_is_preserved() {
    let parsed = parse_server_message(
        r#"{"type":"ChainEvent","data":{"event_type":"PositionMigrated","signature":"sig"}}"#,
    )
    .unwrap();

    assert!(matches!(
        parsed,
        ServerMessage::ChainEvent(ChainEventMessage::Unknown)
    ));
}

#[test]
fn chain_event_identity_requires_instruction_index() {
    assert!(parse_server_message(
        r#"{"type":"ChainEvent","data":{"event_type":"PositionSettled","signature":"sig","slot":1,"position":"position"}}"#,
    ).is_err());
    let identified = parse_server_message(
        r#"{"type":"ChainEvent","data":{"event_type":"PositionSettled","signature":"sig","instruction_index":3,"slot":1,"position":"position"}}"#,
    )
    .unwrap();

    let ServerMessage::ChainEvent(identified) = identified else {
        panic!("expected identified chain event");
    };
    let identity = identified.identity().expect("canonical identity");
    assert_eq!(identity.signature, "sig");
    assert_eq!(identity.instruction_index, 3);
}

#[test]
fn future_payload_of_known_error_stays_usable() {
    // A future cap variant lands in CapError::Unknown with its payload intact.
    let parsed = parse_server_message(
        r#"{"type":"Error","data":{"type":"Cap","data":{"future_cap":{"limit":1}}}}"#,
    )
    .unwrap();
    assert!(matches!(
        parsed,
        ServerMessage::Error(ServerError::Cap(CapError::Unknown(_)))
    ));

    // A known error whose payload no longer matches at all still degrades to
    // UnknownServerError instead of dropping the frame.
    let parsed = parse_server_message(
        r#"{"type":"Error","data":{"type":"RateLimit","data":{"object":"payload"}}}"#,
    )
    .unwrap();
    assert!(matches!(
        parsed,
        ServerMessage::Error(ServerError::Unknown(UnknownServerError {
            error_type,
            ..
        })) if error_type == "RateLimit"
    ));
}

#[test]
fn unknown_nested_rate_limit_reason_is_preserved() {
    let parsed = parse_server_message(
        r#"{"type":"Error","data":{"type":"RateLimit","data":"future_bucket"}}"#,
    )
    .unwrap();

    assert!(matches!(
        parsed,
        ServerMessage::Error(ServerError::RateLimit(RateLimitReason::Unknown))
    ));
}

#[test]
fn unknown_request_error_keeps_request_id() {
    let request_id = Uuid::new_v4();
    let parsed = parse_server_message(&format!(
        r#"{{"type":"RequestError","data":{{"request_id":"{request_id}","error":{{"type":"SomeFutureError"}}}}}}"#,
    ))
    .unwrap();
    match parsed {
        ServerMessage::RequestError(env) => {
            assert_eq!(env.request_id, request_id);
            match env.error {
                ServerError::Unknown(unknown) => {
                    assert_eq!(unknown.error_type, "SomeFutureError");
                    assert!(unknown.raw_json.contains("RequestError"));
                }
                _ => panic!("Expected Unknown error"),
            }
        }
        _ => panic!("Expected RequestError"),
    }
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
    assert_eq!(request.scope, MakerQuoteScope::Live);
    assert_eq!(request.limit, None);
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
        market: MarketId::new("market"),
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
                "pda": "Pos111",
                "market": "Mkt111",
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
    assert_eq!(data.unrenderable[0].pda, "Pos111");
    assert_eq!(data.unrenderable[0].market, MarketId::new("Mkt111"));
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
fn maker_positions_parses_without_the_unrenderable_list() {
    let raw = json!({
        "type": "MakerPositions",
        "data": {
            "request_id": Uuid::new_v4(),
            "positions": [],
            "has_more": false
        }
    });

    let parsed: ServerMessage = serde_json::from_value(raw).unwrap();
    let ServerMessage::MakerPositions(data) = parsed else {
        panic!("expected MakerPositions");
    };
    assert!(data.unrenderable.is_empty());
}

#[test]
fn reconciliation_status_wire_names_are_stable() {
    let parse = |raw: &str| -> PositionReconciliationStatus {
        serde_json::from_str(&format!("\"{raw}\"")).unwrap()
    };
    assert_eq!(
        parse("orphan_pending"),
        PositionReconciliationStatus::OrphanPending
    );
    assert_eq!(
        parse("orphaned_closed"),
        PositionReconciliationStatus::OrphanedClosed
    );
    assert_eq!(
        parse("something_the_server_added_later"),
        PositionReconciliationStatus::Unknown
    );
}

#[test]
fn listing_cursors_ride_as_unix_seconds_and_stay_off_the_wire_when_unset() {
    let request = GetMakerPositionsMessage {
        cursor: Some(UNIX_EPOCH + Duration::from_secs(1_800_000_000)),
        cursor_id: Some("PositionPda".to_string()),
        ..Default::default()
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["cursor"], 1_800_000_000);
    assert_eq!(json["cursor_id"], "PositionPda");

    let bare = serde_json::to_value(GetMakerPositionsMessage::default()).unwrap();
    assert!(bare.get("cursor").is_none());
    assert!(bare.get("cursor_id").is_none());

    let quotes = GetMyQuotesMessage {
        scope: MakerQuoteScope::History,
        cursor: Some(UNIX_EPOCH + Duration::from_secs(1_800_000_777)),
        cursor_id: Some("00ff".repeat(16)),
        ..Default::default()
    };
    let json = serde_json::to_value(&quotes).unwrap();
    assert_eq!(json["cursor"], 1_800_000_777);
    let back: GetMyQuotesMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back.cursor, quotes.cursor);
    assert_eq!(back.cursor_id, quotes.cursor_id);

    let missing_scope = serde_json::from_value::<GetMyQuotesMessage>(json!({
        "request_id": Uuid::new_v4(),
        "limit": 5
    }));
    assert!(missing_scope.is_err(), "scope is required on the wire");
}

#[test]
fn quote_rejected_cap_exceeded_carries_the_cap_error() {
    let raw = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":{"cap_exceeded":"caps_unavailable"}}}"#;
    match serde_json::from_str::<ServerMessage>(raw).unwrap() {
        ServerMessage::QuoteRejected(data) => {
            assert_eq!(
                data.reason,
                QuoteRejectReason::CapExceeded(CapError::CapsUnavailable)
            );
        }
        _ => panic!("Expected QuoteRejected"),
    }

    let structured = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":{"cap_exceeded":{"maker_position_cap_exceeded":{"current":3,"limit":3}}}}}"#;
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

    let plain = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":"invalid_strike"}}"#;
    match serde_json::from_str::<ServerMessage>(plain).unwrap() {
        ServerMessage::QuoteRejected(data) => {
            assert_eq!(data.reason, QuoteRejectReason::InvalidStrike)
        }
        _ => panic!("Expected QuoteRejected"),
    }
}

#[test]
fn future_cap_variant_parses_as_unknown_instead_of_dropping_the_message() {
    let object = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":{"cap_exceeded":{"portfolio_var_cap_exceeded":{"current":"1","limit":"2"}}}}}"#;
    match serde_json::from_str::<ServerMessage>(object).unwrap() {
        ServerMessage::QuoteRejected(data) => {
            assert!(matches!(
                data.reason,
                QuoteRejectReason::CapExceeded(CapError::Unknown(_))
            ));
        }
        _ => panic!("Expected QuoteRejected"),
    }

    let bare = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":{"cap_exceeded":"portfolio_var_cap_exceeded"}}}"#;
    match serde_json::from_str::<ServerMessage>(bare).unwrap() {
        ServerMessage::QuoteRejected(data) => {
            assert!(matches!(
                data.reason,
                QuoteRejectReason::CapExceeded(CapError::Unknown(_))
            ));
        }
        _ => panic!("Expected QuoteRejected"),
    }

    // A reason string this build does not know still parses as `Unknown`.
    let reason = r#"{"type":"QuoteRejected","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","order_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":"quota_exhausted"}}"#;
    match serde_json::from_str::<ServerMessage>(reason).unwrap() {
        ServerMessage::QuoteRejected(data) => assert_eq!(data.reason, QuoteRejectReason::Unknown),
        _ => panic!("Expected QuoteRejected"),
    }
}

#[test]
fn rfq_skipped_carries_optional_cap_detail() {
    let raw = r#"{"type":"RfqSkipped","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","market_id":"m1","quantity":5,"reason":"maker_position_cap_exceeded","cap_detail":{"maker_position_cap_exceeded":{"current":10,"limit":10}}}}"#;
    match serde_json::from_str::<ServerMessage>(raw).unwrap() {
        ServerMessage::RfqSkipped(data) => {
            assert!(matches!(
                data.cap_detail,
                Some(CapError::MakerPositionCapExceeded {
                    current: 10,
                    limit: 10
                })
            ));
        }
        _ => panic!("Expected RfqSkipped"),
    }

    let legacy = r#"{"type":"RfqSkipped","data":{"rfq_id":"00000000-0000-0000-0000-00000000002a","market_id":"m1","quantity":5,"reason":"other"}}"#;
    match serde_json::from_str::<ServerMessage>(legacy).unwrap() {
        ServerMessage::RfqSkipped(data) => assert!(data.cap_detail.is_none()),
        _ => panic!("Expected RfqSkipped"),
    }
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
                    "expires_at": 1,
                    "created_at": 1,
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
                    "position_pda": "position",
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
    assert_eq!(cases.len(), 9);
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
