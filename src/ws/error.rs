use thiserror::Error;

#[derive(Debug, Error)]
pub enum WsClientError {
    #[error("websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
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
pub enum WsTransportConfigError {
    #[error("connect timeout must be non-zero")]
    ZeroConnectTimeout,
    #[error("websocket message and frame limits must be non-zero")]
    ZeroMessageOrFrameLimit,
    #[error("maximum write buffer size must exceed write buffer size")]
    InvalidWriteBufferRange,
}

pub type WsResult<T> = Result<T, WsClientError>;
