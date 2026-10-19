use acta_maker_sdk::http::HttpClient;
use std::time::Duration;

/// `min_quote_validity_seconds` from `GET /api/v1/venue` on the host behind `ws_url`.
pub async fn min_quote_validity(ws_url: &str) -> Result<Duration, Box<dyn std::error::Error>> {
    let base = ws_url
        .trim_end_matches('/')
        .trim_end_matches("/maker/data")
        .trim_end_matches("/maker")
        .trim_end_matches("/vault")
        .replacen("wss://", "https://", 1)
        .replacen("ws://", "http://", 1);
    let venue = HttpClient::new(base)?.venue().await?;
    Ok(Duration::from_secs(venue.min_quote_validity_seconds.into()))
}
