use acta_maker_sdk::http::{HttpClient, Venue};

/// The HTTP API on the host behind `ws_url`.
pub fn client(ws_url: &str) -> Result<HttpClient, Box<dyn std::error::Error>> {
    let base = ws_url
        .trim_end_matches('/')
        .trim_end_matches("/maker/data")
        .trim_end_matches("/maker")
        .trim_end_matches("/vault")
        .replacen("wss://", "https://", 1)
        .replacen("ws://", "http://", 1);
    Ok(HttpClient::new(base)?)
}

/// `GET /api/v1/venue` on the host behind `ws_url`.
pub async fn load(ws_url: &str) -> Result<Venue, Box<dyn std::error::Error>> {
    Ok(client(ws_url)?.venue().await?)
}
