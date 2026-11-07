//! Optional process-local observations at transport boundaries.
use std::sync::OnceLock;

pub enum Event<'a> {
    HttpRequest {
        venue: &'a str,
        endpoint: &'a str,
    },
    HttpStatus {
        venue: &'a str,
        endpoint: &'a str,
        status: u16,
    },
    HttpBytes {
        venue: &'a str,
        endpoint: &'a str,
        bytes: usize,
    },
    Ws {
        venue: &'a str,
        stream: &'a str,
        bytes: usize,
    },
    Rpc {
        method: &'a str,
    },
}
static OBSERVER: OnceLock<fn(Event<'_>)> = OnceLock::new();
pub fn install(observer: fn(Event<'_>)) {
    let _ = OBSERVER.set(observer);
}
pub fn record(event: Event<'_>) {
    if let Some(observer) = OBSERVER.get() {
        observer(event);
    }
}

#[cfg(feature = "http")]
pub async fn send(
    request: reqwest::RequestBuilder,
    venue: &str,
    endpoint: &str,
) -> Result<reqwest::Response, reqwest::Error> {
    record(Event::HttpRequest { venue, endpoint });
    let response = request.send().await;
    record(Event::HttpStatus {
        venue,
        endpoint,
        status: response.as_ref().map_or(0, |r| r.status().as_u16()),
    });
    response
}
#[cfg(feature = "http")]
pub async fn bytes(
    mut response: reqwest::Response,
    venue: &str,
    endpoint: &str,
) -> Result<Vec<u8>, reqwest::Error> {
    let mut count = BodyCount {
        venue,
        endpoint,
        bytes: 0,
    };
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        count.bytes += chunk.len();
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(feature = "http")]
struct BodyCount<'a> {
    venue: &'a str,
    endpoint: &'a str,
    bytes: usize,
}
#[cfg(feature = "http")]
impl Drop for BodyCount<'_> {
    fn drop(&mut self) {
        record(Event::HttpBytes {
            venue: self.venue,
            endpoint: self.endpoint,
            bytes: self.bytes,
        });
    }
}
