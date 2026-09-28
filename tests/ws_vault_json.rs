//! Vault-plane frames, byte-for-byte against the backend's `acta-ws-protocol`
//! tests: `u64` amounts ride as strings, `version`/`slot` as numbers,
//! `permissions` as a plain integer, enum tags in `snake_case`.

use acta_maker_sdk::ws::types::*;
use acta_maker_sdk::{OrderId, VaultDelegatePermissions, VaultOperatorKind, VaultSwapMode};
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
        operator: "operator".to_string(),
        vault_pda: "vault".to_string(),
    });
    let parsed = assert_client_wire(
        &message,
        r#"{"type":"StartVaultAuth","data":{"operator":"operator","vault_pda":"vault"}}"#,
    );
    let ClientMessage::StartVaultAuth(data) = parsed else {
        panic!("expected StartVaultAuth");
    };
    assert_eq!(data.operator, "operator");
    assert_eq!(data.vault_pda, "vault");
    assert_eq!(message.request_id(), None);
}

#[test]
fn vault_swap_quote_and_submit_roundtrip() {
    let quote = ClientMessage::SwapQuote(SwapQuoteData {
        request_id: request_id(),
        vault_pda: "vault".to_string(),
        mode: VaultSwapMode::Rebalance,
        input_mint: "main".to_string(),
        output_mint: "second".to_string(),
        amount: u64::MAX.to_string(),
        slippage_bps: 25,
    });
    let expected = format!(
        r#"{{"type":"SwapQuote","data":{{"request_id":"{REQUEST_ID}","vault_pda":"vault","mode":"rebalance","input_mint":"main","output_mint":"second","amount":"{}","slippage_bps":25}}}}"#,
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
fn swap_quote_slippage_defaults_to_50_bps() {
    let raw = format!(
        r#"{{"type":"SwapQuote","data":{{"request_id":"{REQUEST_ID}","vault_pda":"vault","mode":"recovery","input_mint":"main","output_mint":"second","amount":"1"}}}}"#
    );
    let ClientMessage::SwapQuote(data) = serde_json::from_str(&raw).unwrap() else {
        panic!("expected SwapQuote");
    };
    assert_eq!(data.slippage_bps, 50);
    assert_eq!(data.mode, VaultSwapMode::Recovery);
}

#[test]
fn vault_auth_success_roundtrip() {
    let permissions = VaultDelegatePermissions::from_bits(3);
    let message = ServerMessage::VaultAuthSuccess(VaultAuthSuccessData {
        vault_pda: "vault".to_string(),
        operator: "operator".to_string(),
        operator_kind: VaultOperatorKind::Delegate,
        permissions,
    });
    let parsed = assert_server_wire(
        &message,
        r#"{"type":"VaultAuthSuccess","data":{"vault_pda":"vault","operator":"operator","operator_kind":"delegate","permissions":3}}"#,
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
fn vault_enums_decode_unknown_tags() {
    let raw = r#"{"type":"VaultAuthSuccess","data":{"vault_pda":"vault","operator":"operator","operator_kind":"custodian","permissions":128}}"#;
    let ServerMessage::VaultAuthSuccess(data) = parse_server_message(raw).unwrap() else {
        panic!("expected VaultAuthSuccess");
    };
    assert_eq!(data.operator_kind, VaultOperatorKind::Unknown);
    assert_eq!(data.permissions.bits(), 128);
    assert!(!data.permissions.contains(VaultDelegatePermissions::ALL));
}

#[test]
fn vault_projection_advanced_carries_numeric_version_and_slot() {
    let message = ServerMessage::VaultProjectionAdvanced(VaultProjectionAdvancedData {
        vault_pda: "vault".to_string(),
        version: 7,
        slot: 42,
        transaction_index: 3,
        signature: "sig".to_string(),
        event_index: 1,
    });
    let parsed = assert_server_wire(
        &message,
        r#"{"type":"VaultProjectionAdvanced","data":{"vault_pda":"vault","version":7,"slot":42,"transaction_index":3,"signature":"sig","event_index":1}}"#,
    );
    let ServerMessage::VaultProjectionAdvanced(data) = parsed else {
        panic!("expected VaultProjectionAdvanced");
    };
    assert_eq!((data.version, data.slot), (7, 42));
    assert_eq!(message.request_id(), None);
}

#[test]
fn vault_swap_results_roundtrip_without_claiming_chain_confirmation() {
    let quote = ServerMessage::SwapQuoteResult(SwapQuoteResultData {
        request_id: request_id(),
        execution_id: execution_id(),
        mode: VaultSwapMode::Recovery,
        tx_base64: "unsigned".to_string(),
        in_amount: "42".to_string(),
        quoted_out_amount: "41".to_string(),
        minimum_out_amount: "40".to_string(),
        valid_until: "123".to_string(),
        route_hash: "ab".repeat(32),
    });
    let expected = format!(
        r#"{{"type":"SwapQuoteResult","data":{{"request_id":"{REQUEST_ID}","execution_id":"{EXECUTION_ID_HEX}","mode":"recovery","tx_base64":"unsigned","in_amount":"42","quoted_out_amount":"41","minimum_out_amount":"40","valid_until":"123","route_hash":"{}"}}}}"#,
        "ab".repeat(32)
    );
    let ServerMessage::SwapQuoteResult(data) = assert_server_wire(&quote, &expected) else {
        panic!("expected SwapQuoteResult");
    };
    assert_eq!(data.execution_id, execution_id());
    assert_eq!(data.minimum_out_amount, "40");
    assert_eq!(quote.request_id(), Some(request_id()));

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
