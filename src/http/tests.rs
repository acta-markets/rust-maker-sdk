use std::io::{Read as _, Write as _};
use std::net::TcpListener;

use serde_json::Value;

use super::types::{MarketsResponse, TokensResponse, VaultPositionsResponse, VaultsResponse};
use super::{
    HttpClient, HttpError, MarketFilter, TokenProgramKind, VaultExposure, VaultPositionRole,
    VaultPositionStatus, VaultPositionType, Venue,
};
use crate::ws::types::MarketStatus;

const VAULTS: &str = include_str!("../../tests/fixtures/http_vaults.json");
const EXPOSURE: &str = include_str!("../../tests/fixtures/http_exposure.json");
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
    let [full, minimal] = response.vaults.as_slice() else {
        panic!("two vaults");
    };
    assert_eq!(full.second_token_program, TokenProgramKind::Token2022);
    assert_eq!(full.phase, "active");
    assert_eq!(full.capacity.remaining.as_deref(), Some("860"));
    assert_eq!(full.tvl_usd_1e9.as_deref(), Some("18000"));
    assert_eq!(full.depositors_in_generation, Some(11));
    let premium = full.premium.as_deref().unwrap();
    assert_eq!(premium[0].generation.paid.as_deref(), Some("7"));
    assert!(premium[1].generation.paid.is_none());
    assert_eq!(full.fees.as_ref().unwrap().realized_fee_value, "6");
    let performance = full.performance.as_ref().unwrap();
    assert!(performance.covered);
    assert_eq!(performance.capital_generation, full.capital_generation);
    assert!(performance.returns.d7.is_none());
    assert_eq!(performance.returns.d30.as_ref().unwrap().return_pct, 1.5);
    assert_eq!(performance.cycles_completed, Some(4));
    assert_eq!(
        full.metadata
            .as_ref()
            .map(|m| (m.name.as_str(), m.twitter.as_deref())),
        Some(("SOL vault", Some("acta_markets")))
    );

    assert_eq!(minimal.metadata, None);
    assert!(minimal.main_balance.is_none() && minimal.deposit_window_set_ts.is_none());
    assert_eq!(minimal.capacity.remaining, None);
    assert!(minimal.premium.is_none() && minimal.fees.is_none() && minimal.tvl_usd_1e9.is_none());
    let performance = minimal.performance.as_ref().unwrap();
    assert!(!performance.covered);
    assert!(performance.returns.all.is_none() && performance.cycles_completed.is_none());
}

#[test]
fn vault_exposure_round_trips_the_backend_shape() {
    let exposure: VaultExposure = round_trip(EXPOSURE);
    assert_eq!(exposure.as_of, "1702600005");
    let [call, put, unpriced] = exposure.positions.as_slice() else {
        panic!("three positions");
    };
    assert_eq!(call.role, VaultPositionRole::Writer);
    assert_eq!(call.itm, Some(true));
    assert_eq!(call.intrinsic.as_deref(), Some("-20000000"));
    assert_eq!(put.position_type, VaultPositionType::CashSecuredPut);
    assert_eq!(put.status, VaultPositionStatus::Funded);
    assert!(unpriced.spot.is_none() && unpriced.quote_decimals.is_none());
    assert_eq!(
        exposure.totals[0].outstanding_intrinsic.as_deref(),
        Some("-10000000")
    );
    assert!(exposure.totals[1].notional_at_strike.is_none());
}

#[test]
fn markets_round_trip_the_backend_shape() {
    let response: MarketsResponse = round_trip(MARKETS);
    let [finalized, active] = response.markets.as_slice() else {
        panic!("two markets");
    };
    assert_eq!(finalized.status, MarketStatus::Finalized);
    assert_eq!(
        finalized.settlement_price.map(|price| price.value()),
        Some(150_000_000_000)
    );
    assert_eq!(active.status, MarketStatus::Active);
    assert!(active.settlement_price.is_none());
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
    let response: TokensResponse = round_trip(TOKENS);
    let [usdc, sol] = response.tokens.as_slice() else {
        panic!("two tokens");
    };
    assert_eq!(sol.strike_tick.map(|tick| tick.value()), Some(500_000_000));
    assert_eq!(sol.oracle_feed_ids.len(), 1);
    assert!(sol.size_rule.is_some());
    assert_eq!(sol.quote_mints, std::slice::from_ref(&usdc.mint));
    assert_eq!(usdc.symbol, "USDC");
    assert!(usdc.size_rule.is_none() && usdc.strike_tick.is_none() && usdc.quote_mints.is_empty());
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
    value["vaults"][0]["performance"]["new_metric"] = 1.into();
    let response: VaultsResponse = serde_json::from_value(value).unwrap();
    assert_eq!(
        response.vaults[0].main_token_program,
        TokenProgramKind::Unknown
    );
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
        ("200 OK", EXPOSURE.to_string()),
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
    assert!(vault.metadata.is_none());
    assert_eq!(
        client
            .markets(&MarketFilter::default())
            .await
            .unwrap()
            .len(),
        2
    );
    let tokens = client.tokens().await.unwrap();
    assert_eq!(
        tokens["So11111111111111111111111111111111111111112"].symbol,
        "SOL"
    );
    assert_eq!(tokens.len(), 2);
    assert_eq!(client.venue().await.unwrap().settlement_buffer_seconds, 90);
    let exposure = client
        .vault_exposure("11111111111111111111111111111111")
        .await
        .unwrap();
    assert_eq!(exposure.positions.len(), 3);
    assert!(matches!(
        client.vault_exposure("../markets").await,
        Err(HttpError::Address(_))
    ));
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
            "/api/v1/vaults/11111111111111111111111111111111/exposure",
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

#[tokio::test]
async fn reference_reads_revalidate_with_etag() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for status in ["200 OK", "304 Not Modified"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap();
            requests.push(String::from_utf8_lossy(&request[..read]).to_lowercase());
            let body = if status == "200 OK" { MARKETS } else { "" };
            let reply = format!(
                "HTTP/1.1 {status}\r\netag: \"m1\"\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
        requests
    });
    let client = HttpClient::new(base).unwrap();
    let filter = MarketFilter {
        active: Some(true),
        is_put: Some(false),
        ..MarketFilter::default()
    };
    let first = client.markets(&filter).await.unwrap();
    let second = client.markets(&filter).await.unwrap();
    assert_eq!(first, second);
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("get /api/v1/markets?is_put=false&active=true "));
    assert!(!requests[0].contains("if-none-match"));
    assert!(requests[1].contains("if-none-match: \"m1\""));
}
