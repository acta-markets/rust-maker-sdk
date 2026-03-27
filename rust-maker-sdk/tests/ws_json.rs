use acta_maker_sdk::{Decimals, Nonce, OrderId, Price, RfqVersion, Strike};
use proptest::prelude::*;

use acta_maker_sdk::ws::types::*;
use serde_json::json;
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[test]
fn auth_success_parses_optional_expires_at() {
    let raw_with_null = json!({
        "type": "AuthSuccess",
        "data": {
            "session_id": "session-1",
            "expires_at": null
        }
    });
    let parsed_with_null: ServerMessage = serde_json::from_value(raw_with_null).unwrap();
    match parsed_with_null {
        ServerMessage::AuthSuccess(data) => {
            assert_eq!(data.session_id, "session-1");
            assert_eq!(data.expires_at, None);
        }
        _ => panic!("expected AuthSuccess"),
    }

    let raw_with_expiry = json!({
        "type": "AuthSuccess",
        "data": {
            "session_id": "session-2",
            "expires_at": 1_710_086_400
        }
    });
    let parsed_with_expiry: ServerMessage = serde_json::from_value(raw_with_expiry).unwrap();
    match parsed_with_expiry {
        ServerMessage::AuthSuccess(data) => {
            assert_eq!(data.session_id, "session-2");
            assert_eq!(
                data.expires_at,
                Some(UNIX_EPOCH + Duration::from_secs(1_710_086_400))
            );
        }
        _ => panic!("expected AuthSuccess"),
    }
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

// --- Tests moved from server.rs inline tests ---

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

// --- Tests moved from client.rs inline tests ---

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

// --- New tests ---

#[test]
fn welcome_roundtrip() {
    let msg = ServerMessage::Welcome(WelcomeData {
        protocol_version: "1.0.0".to_string(),
        server_version: "0.5.0".to_string(),
        min_supported_version: "1.0.0".to_string(),
        enabled_features: vec!["quote_expired".to_string()],
        server_time_unix_ms: Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000)),
    });
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::Welcome(data) => {
            assert_eq!(data.protocol_version, "1.0.0");
            assert!(data.server_time_unix_ms.is_some());
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
        }
        _ => panic!("expected RfqBroadcast"),
    }
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
        valid_until: UNIX_EPOCH + Duration::from_secs(999),
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
            valid_until: UNIX_EPOCH + Duration::from_secs(valid_until),
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
                prop_assert_eq!(decoded.valid_until, UNIX_EPOCH + Duration::from_secs(valid_until));
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

// --- New protocol message tests ---

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
        reason: QuoteRejectReason::CapExceeded,
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
        valid_until: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
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
        valid_until: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        nonce: Nonce::new(1),
        order_id: OrderId([0xaa; 32]),
        signature: "sig1".to_string(),
    };
    let q2 = QuoteMessage {
        rfq_id: Uuid::new_v4(),
        strike: Strike::new(110_000_000_000),
        price: Price::new(3_000_000),
        valid_until: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
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
