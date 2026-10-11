use std::io::{Read as _, Write as _};
use std::net::TcpListener;

use serde_json::Value;

use super::types::{MarketsResponse, VaultPositionsResponse, VaultsResponse};
use super::{
    HttpClient, HttpError, TokenProgramKind, Tokens, VaultPerformance, VaultPositionRole,
    VaultPositionStatus, VaultPositionType, Venue,
};

const VAULTS: &str = include_str!("../../tests/fixtures/http_vaults.json");
const MARKETS: &str = include_str!("../../tests/fixtures/http_markets.json");
const POSITIONS: &str = include_str!("../../tests/fixtures/http_vault_positions.json");
const VENUE: &str = include_str!("../../tests/fixtures/http_venue.json");
const TOKENS: &str = include_str!("../../tests/fixtures/http_tokens.json");

fn round_trip<T: serde::de::DeserializeOwned + serde::Serialize>(json: &str) -> T {
    let decoded: T = serde_json::from_str(json).unwrap();
    let expected: Value = serde_json::from_str(json).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), expected);
    decoded
}

#[test]
fn vaults_round_trip_the_backend_shape() {
    let response: VaultsResponse = round_trip(VAULTS);
    let [idle, active] = response.vaults.as_slice() else {
        panic!("two vaults");
    };
    assert_eq!(idle.second_token_program, TokenProgramKind::Token2022);
    assert_eq!(idle.main_balance.as_deref(), Some("900"));
    assert!(idle.accepting_deposits && !idle.equity_computed);
    assert!(matches!(
        idle.performance,
        VaultPerformance::ShortTrackRecord(_)
    ));
    assert_eq!(active.phase, "active");
    assert_eq!(active.active_expiry, "1767225600");
    assert!(active.main_balance.is_none());
    assert!(matches!(
        active.performance,
        VaultPerformance::Annualized { annualized_return_pct, .. } if annualized_return_pct > 54.0
    ));
}

#[test]
fn markets_round_trip_the_backend_shape() {
    let response: MarketsResponse = round_trip(MARKETS);
    assert_eq!(response.markets.len(), 2);
    assert_eq!(response.markets[1].settlement_price, Some(150_000_000_000));
}

#[test]
fn venue_round_trips_the_backend_shape() {
    let venue: Venue = round_trip(VENUE);
    assert_eq!(venue.min_quote_validity_seconds, 100);
    assert_eq!(venue.max_rfq_timeout_seconds, 120);
    assert_eq!(venue.max_quotes_per_rfq, Some(50));
}

#[test]
fn tokens_round_trip_the_backend_shape() {
    let tokens: Tokens = round_trip(TOKENS);
    let [sol] = tokens.underlyings.as_slice() else {
        panic!("one underlying");
    };
    assert_eq!(sol.strike_tick.map(|tick| tick.value()), Some(500_000_000));
    assert_eq!(sol.oracle_feed_ids.len(), 1);
    let [usdc] = tokens.quotes_by_underlying[&sol.token.mint].as_slice() else {
        panic!("one quote");
    };
    assert!(usdc.strike_tick.is_none());
    assert_eq!(usdc.token.symbol, "USDC");
}

#[test]
fn ws_token_info_keeps_the_published_shape_and_ignores_new_fields() {
    let tokens: Value = serde_json::from_str(TOKENS).unwrap();
    let crate::ws::types::TokenInfo {
        mint,
        decimals,
        size_rule: _,
        symbol,
    } = serde_json::from_value(tokens["underlyings"][0].clone()).unwrap();
    assert_eq!(
        (mint.as_str(), u8::from(decimals), symbol.as_str()),
        ("So11111111111111111111111111111111111111112", 9, "SOL")
    );
}

#[test]
fn vault_positions_round_trip_the_backend_shape() {
    let response: VaultPositionsResponse = round_trip(POSITIONS);
    let [put, call] = response.positions.as_slice() else {
        panic!("two positions");
    };
    assert_eq!(put.role, VaultPositionRole::Writer);
    assert_eq!(put.position_type, VaultPositionType::CashSecuredPut);
    assert_eq!(put.status, VaultPositionStatus::Funded);
    assert_eq!(call.role, VaultPositionRole::Holder);
    assert_eq!(call.is_otm, Some(true));
    assert!(response.next_cursor.is_some());
}

#[test]
fn unknown_enum_values_decode() {
    let mut value: Value = serde_json::from_str(VAULTS).unwrap();
    value["vaults"][0]["main_token_program"] = "token_2099".into();
    value["vaults"][0]["performance"] = serde_json::json!({"status": "new_kind"});
    let response: VaultsResponse = serde_json::from_value(value).unwrap();
    assert_eq!(
        response.vaults[0].main_token_program,
        TokenProgramKind::Unknown
    );
    assert_eq!(response.vaults[0].performance, VaultPerformance::Unknown);
}

fn serve(responses: Vec<(&'static str, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut paths = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let line = String::from_utf8_lossy(&request[..read]);
            paths.push(line.split_whitespace().nth(1).unwrap().to_string());
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
        paths
    });
    (base, handle)
}

#[tokio::test]
async fn client_reads_public_routes() {
    let vaults: Value = serde_json::from_str(VAULTS).unwrap();
    let detail = vaults["vaults"][1].to_string();
    let (base, server) = serve(vec![
        ("200 OK", VAULTS.to_string()),
        ("200 OK", detail),
        ("200 OK", MARKETS.to_string()),
        ("200 OK", TOKENS.to_string()),
        ("200 OK", VENUE.to_string()),
        (
            "404 Not Found",
            r#"{"error":"Vault not found"}"#.to_string(),
        ),
    ]);
    let client = HttpClient::new(base).unwrap();
    assert_eq!(client.vaults().await.unwrap().len(), 2);
    let vault = client
        .vault("11111111111111111111111111111111")
        .await
        .unwrap();
    assert_eq!(vault.open_positions, "2");
    assert_eq!(client.markets().await.unwrap().len(), 2);
    assert_eq!(client.tokens().await.unwrap().underlyings.len(), 1);
    assert_eq!(client.venue().await.unwrap().settlement_buffer_seconds, 90);
    let missing = client.vault("11111111111111111111111111111111").await;
    assert!(matches!(
        missing,
        Err(HttpError::Status { status: 404, .. })
    ));
    assert!(matches!(
        client.vault("../markets").await,
        Err(HttpError::Address(_))
    ));
    assert_eq!(
        server.join().unwrap(),
        [
            "/api/v1/vaults",
            "/api/v1/vaults/11111111111111111111111111111111",
            "/api/v1/markets",
            "/api/v1/tokens",
            "/api/v1/venue",
            "/api/v1/vaults/11111111111111111111111111111111",
        ]
    );
}

#[tokio::test]
async fn vault_positions_follow_the_cursor() {
    let mut last: Value = serde_json::from_str(POSITIONS).unwrap();
    last["positions"] = serde_json::json!([last["positions"][0].clone()]);
    last["next_cursor"] = Value::Null;
    let (base, server) = serve(vec![
        ("200 OK", POSITIONS.to_string()),
        ("200 OK", last.to_string()),
    ]);
    let client = HttpClient::new(base).unwrap();
    let positions = client
        .vault_positions("11111111111111111111111111111111")
        .await
        .unwrap();
    assert_eq!(positions.len(), 3);
    assert!(matches!(
        client.vault_positions("../markets").await,
        Err(HttpError::Address(_))
    ));
    let path = "/api/v1/vaults/11111111111111111111111111111111/positions?settled=false&limit=200";
    assert_eq!(
        server.join().unwrap(),
        [
            path.to_string(),
            format!("{path}&cursor=412:3:1:7cVfgArCheMR6Cs4t6vz5rfnqd56vZq4ndaBrY5xkxXy"),
        ]
    );
}
