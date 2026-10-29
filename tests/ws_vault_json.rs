//! Vault-plane frames, byte-for-byte against the backend's `acta-ws-protocol`
//! tests: `u64` amounts ride as strings, `version`/`slot` as numbers,
//! `permissions` as a plain integer, enum tags in `snake_case`.

use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{
    OrderId, VaultDelegatePermissions, VaultOperatorKind, VaultStagingStep, VaultSwapMode,
};
use uuid::Uuid;

const REQUEST_ID: &str = "0f2c7b6e-3f1a-4d3e-9b1e-1c2d3e4f5a6b";
const EXECUTION_ID_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

fn request_id() -> Uuid {
    Uuid::parse_str(REQUEST_ID).unwrap()
}

fn execution_id() -> OrderId {
    OrderId::new([7; 32])
}

fn assert_client_wire(message: &ClientMessage, expected: &str) -> ClientMessage {
    assert_eq!(serde_json::to_string(message).unwrap(), expected);
    serde_json::from_str(expected).unwrap()
}

fn assert_server_wire(message: &ServerMessage, expected: &str) -> ServerMessage {
    assert_eq!(serde_json::to_string(message).unwrap(), expected);
    parse_server_message(expected).unwrap()
}

#[test]
fn start_vault_auth_roundtrip() {
    let message = ClientMessage::StartVaultAuth(StartVaultAuthData {
        operator: acta_maker_sdk::Address32::from_base58(
            "TvHro9eZ3AMmmTr83uL1tJDubsTad4dj4rEbK1jC2Pk",
        )
        .unwrap(),
        vault_pda: acta_maker_sdk::Address32::from_base58(
            "GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY",
        )
        .unwrap(),
    });
    let parsed = assert_client_wire(
        &message,
        r#"{"type":"StartVaultAuth","data":{"operator":"TvHro9eZ3AMmmTr83uL1tJDubsTad4dj4rEbK1jC2Pk","vault_pda":"GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY"}}"#,
    );
    let ClientMessage::StartVaultAuth(data) = parsed else {
        panic!("expected StartVaultAuth");
    };
    assert_eq!(
        data.operator,
        acta_maker_sdk::Address32::from_base58("TvHro9eZ3AMmmTr83uL1tJDubsTad4dj4rEbK1jC2Pk")
            .unwrap()
    );
    assert_eq!(
        data.vault_pda,
        acta_maker_sdk::Address32::from_base58("GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY")
            .unwrap()
    );
    assert_eq!(message.request_id(), None);
}

#[test]
fn vault_swap_quote_and_submit_roundtrip() {
    let quote = ClientMessage::SwapQuote(SwapQuoteData {
        request_id: request_id(),
        vault_pda: acta_maker_sdk::Address32::from_base58(
            "GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY",
        )
        .unwrap(),
        mode: VaultSwapMode::Rebalance,
        direction: VaultSwapDirection::MainToSecond,
        amount: u64::MAX.to_string(),
        slippage_bps: 25,
    });
    let expected = format!(
        r#"{{"type":"SwapQuote","data":{{"request_id":"{REQUEST_ID}","vault_pda":"GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY","mode":"rebalance","direction":"main_to_second","amount":"{}","slippage_bps":25}}}}"#,
        u64::MAX
    );
    let ClientMessage::SwapQuote(data) = assert_client_wire(&quote, &expected) else {
        panic!("expected SwapQuote");
    };
    assert_eq!(data.request_id, request_id());
    assert_eq!(data.amount, u64::MAX.to_string());
    assert_eq!(data.slippage_bps, 25);
    assert_eq!(quote.request_id(), Some(request_id()));

    let submit = ClientMessage::SwapSubmit(SwapSubmitData {
        request_id: request_id(),
        execution_id: execution_id(),
        signed_tx_base64: "signed".to_string(),
    });
    let expected = format!(
        r#"{{"type":"SwapSubmit","data":{{"request_id":"{REQUEST_ID}","execution_id":"{EXECUTION_ID_HEX}","signed_tx_base64":"signed"}}}}"#
    );
    let ClientMessage::SwapSubmit(data) = assert_client_wire(&submit, &expected) else {
        panic!("expected SwapSubmit");
    };
    assert_eq!(data.execution_id, execution_id());
}

#[test]
fn swap_quote_requires_slippage() {
    let raw = format!(
        r#"{{"type":"SwapQuote","data":{{"request_id":"{REQUEST_ID}","vault_pda":"GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY","mode":"recovery","direction":"second_to_main","amount":"1"}}}}"#
    );
    assert!(serde_json::from_str::<ClientMessage>(&raw).is_err());
}

#[test]
fn vault_auth_success_roundtrip() {
    let permissions = VaultDelegatePermissions::from_bits(3);
    let message = ServerMessage::VaultAuthSuccess(VaultAuthSuccessData {
        vault_pda: acta_maker_sdk::Address32::from_base58(
            "GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY",
        )
        .unwrap(),
        operator: acta_maker_sdk::Address32::from_base58(
            "TvHro9eZ3AMmmTr83uL1tJDubsTad4dj4rEbK1jC2Pk",
        )
        .unwrap(),
        operator_kind: VaultOperatorKind::Delegate,
        permissions,
    });
    let parsed = assert_server_wire(
        &message,
        r#"{"type":"VaultAuthSuccess","data":{"vault_pda":"GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY","operator":"TvHro9eZ3AMmmTr83uL1tJDubsTad4dj4rEbK1jC2Pk","operator_kind":"delegate","permissions":3}}"#,
    );
    let ServerMessage::VaultAuthSuccess(data) = parsed else {
        panic!("expected VaultAuthSuccess");
    };
    assert_eq!(data.permissions, permissions);
    assert!(
        data.permissions
            .contains(VaultDelegatePermissions::OPEN_TAKER)
    );
    assert!(
        data.permissions
            .contains(VaultDelegatePermissions::SIGN_MAKER_QUOTES)
    );
    assert!(
        !data
            .permissions
            .contains(VaultDelegatePermissions::FUND_MAKER)
    );
    assert_eq!(data.operator_kind, VaultOperatorKind::Delegate);
    assert_eq!(message.request_id(), None);
}

#[test]
fn vault_enums_reject_unknown_tags() {
    let raw = r#"{"type":"VaultAuthSuccess","data":{"vault_pda":"GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY","operator":"TvHro9eZ3AMmmTr83uL1tJDubsTad4dj4rEbK1jC2Pk","operator_kind":"custodian","permissions":128}}"#;
    assert!(parse_server_message(raw).is_err());
}

#[test]
fn vault_projection_advanced_carries_numeric_version_and_slot() {
    let message = ServerMessage::VaultProjectionAdvanced(VaultProjectionAdvancedData {
        vault_pda: acta_maker_sdk::Address32::from_base58(
            "GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY",
        )
        .unwrap(),
        version: 7,
        slot: 42,
        transaction_index: 3,
        signature: "sig".to_string(),
        event_index: 1,
    });
    let parsed = assert_server_wire(
        &message,
        r#"{"type":"VaultProjectionAdvanced","data":{"vault_pda":"GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY","version":7,"slot":42,"transaction_index":3,"signature":"sig","event_index":1}}"#,
    );
    let ServerMessage::VaultProjectionAdvanced(data) = parsed else {
        panic!("expected VaultProjectionAdvanced");
    };
    assert_eq!((data.version, data.slot), (7, 42));
    assert_eq!(message.request_id(), None);
}

#[test]
fn vault_catalog_changed_names_the_vault() {
    let message = ServerMessage::VaultCatalogChanged(VaultCatalogChangedData {
        vault_pda: acta_maker_sdk::Address32::from_base58(
            "GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY",
        )
        .unwrap(),
    });
    let parsed = assert_server_wire(
        &message,
        r#"{"type":"VaultCatalogChanged","data":{"vault_pda":"GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY"}}"#,
    );
    let ServerMessage::VaultCatalogChanged(data) = parsed else {
        panic!("expected VaultCatalogChanged");
    };
    assert_eq!(
        data.vault_pda,
        acta_maker_sdk::Address32::from_base58("GYVb4hWw8D22pkScWSZZB1QjT7jmuFkPCR1a9DCe1GjY")
            .unwrap()
    );
    assert_eq!(message.request_id(), None);
}

#[test]
fn vault_swap_results_roundtrip_without_claiming_chain_confirmation() {
    let quote = ServerMessage::SwapQuoteResult(SwapQuoteResultData {
        request_id: request_id(),
        execution_id: execution_id(),
        mode: VaultSwapMode::Recovery,
        tx_base64: "unsigned".to_string(),
        last_valid_block_height: 77,
        in_amount: "42".to_string(),
        quoted_out_amount: "41".to_string(),
        minimum_out_amount: "40".to_string(),
        valid_until: "123".to_string(),
        staging: None,
    });
    let expected = format!(
        r#"{{"type":"SwapQuoteResult","data":{{"request_id":"{REQUEST_ID}","execution_id":"{EXECUTION_ID_HEX}","mode":"recovery","tx_base64":"unsigned","last_valid_block_height":77,"in_amount":"42","quoted_out_amount":"41","minimum_out_amount":"40","valid_until":"123","staging":null}}}}"#
    );
    let ServerMessage::SwapQuoteResult(data) = assert_server_wire(&quote, &expected) else {
        panic!("expected SwapQuoteResult");
    };
    assert_eq!(data.execution_id, execution_id());
    assert_eq!(data.minimum_out_amount, "40");
    assert_eq!(quote.request_id(), Some(request_id()));
    let older = expected.replace(
        r#""valid_until":"123""#,
        r#""valid_until":"123","main_price_update":"M","second_price_update":"S""#,
    );
    assert!(serde_json::from_str::<ServerMessage>(&older).is_ok());
    let staged = expected.replace(r#""staging":null"#, r#""staging":"unstage""#);
    let ServerMessage::SwapQuoteResult(data) = serde_json::from_str(&staged).unwrap() else {
        panic!("expected SwapQuoteResult");
    };
    assert_eq!(data.staging, Some(VaultStagingStep::Unstage));

    let submit = ServerMessage::SwapSubmitResult(SwapSubmitResultData {
        request_id: request_id(),
        execution_id: execution_id(),
    });
    let expected = format!(
        r#"{{"type":"SwapSubmitResult","data":{{"request_id":"{REQUEST_ID}","execution_id":"{EXECUTION_ID_HEX}"}}}}"#
    );
    assert!(!expected.contains("signature"));
    assert!(matches!(
        assert_server_wire(&submit, &expected),
        ServerMessage::SwapSubmitResult(SwapSubmitResultData { execution_id, .. })
            if execution_id == self::execution_id()
    ));
    assert_eq!(submit.request_id(), Some(request_id()));
}

#[test]
fn vault_channel_and_endpoint_tags() {
    assert_eq!(
        serde_json::to_string(&WsChannel::Vaults).unwrap(),
        r#""vaults""#
    );
    assert_eq!(
        serde_json::from_str::<WsChannel>(r#""vaults""#).unwrap(),
        WsChannel::Vaults
    );
    assert_eq!(
        serde_json::to_string(&WsEndpointKind::Vault).unwrap(),
        r#""vault""#
    );
    assert_eq!(
        serde_json::from_str::<WsEndpointKind>(r#""vault""#).unwrap(),
        WsEndpointKind::Vault
    );
}
