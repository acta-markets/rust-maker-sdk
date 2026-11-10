use acta_maker_sdk::ws::types::{ErrorAction, ServerError, WsErrorCode};

#[test]
fn error_actions_match_the_server_catalog() {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/error_actions.json")).unwrap();
    for (code, action) in catalog["codes"].as_object().unwrap() {
        let code: WsErrorCode = serde_json::from_value(code.as_str().into()).unwrap();
        let action: ErrorAction = serde_json::from_value(action.clone()).unwrap();
        assert_eq!(code.action(), action, "{code:?}");
    }
    for entry in catalog["typed"].as_array().unwrap() {
        let error: ServerError = serde_json::from_value(entry["error"].clone()).unwrap();
        let action: ErrorAction = serde_json::from_value(entry["action"].clone()).unwrap();
        assert_eq!(error.action(), action, "{}", entry["error"]);
    }
}
