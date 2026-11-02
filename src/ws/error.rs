use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WsClientError {
    #[error("websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unreadable frame: {0}")]
    UnreadableFrame(Box<UnreadableFrame>),
    #[error("read timeout")]
    Timeout,
    #[error("websocket connect timed out")]
    ConnectTimeout,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error(transparent)]
    InvalidTransportConfig(#[from] WsTransportConfigError),
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WsTransportConfigError {
    #[error("connect timeout must be non-zero")]
    ZeroConnectTimeout,
    #[error("websocket message and frame limits must be non-zero")]
    ZeroMessageOrFrameLimit,
    #[error("maximum write buffer size must exceed write buffer size")]
    InvalidWriteBufferRange,
}

pub type WsResult<T> = Result<T, WsClientError>;

/// A server frame this SDK version cannot decode. `frame_type` and `request_id` come from
/// a lenient read of the raw JSON, when it is JSON at all.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}: {error}", frame_type.as_deref().unwrap_or("unknown type"))]
pub struct UnreadableFrame {
    pub frame_type: Option<String>,
    pub request_id: Option<uuid::Uuid>,
    pub error: String,
}

impl UnreadableFrame {
    pub(crate) fn from_raw(raw: &[u8], error: &serde_json::Error) -> Self {
        let value = serde_json::from_slice::<serde_json::Value>(raw).ok();
        let frame_type = value
            .as_ref()
            .and_then(|value| value.get("type")?.as_str())
            .map(str::to_owned);
        let request_id = value
            .as_ref()
            .and_then(|value| value.get("data")?.get("request_id")?.as_str())
            .and_then(|id| uuid::Uuid::parse_str(id).ok());
        Self {
            frame_type,
            request_id,
            error: error.to_string(),
        }
    }
}
