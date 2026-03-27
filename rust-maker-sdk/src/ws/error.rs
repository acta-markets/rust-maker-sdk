use thiserror::Error;

#[derive(Debug, Error)]
pub enum WsClientError {
    #[error("websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("read timeout")]
    Timeout,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub type WsResult<T> = Result<T, WsClientError>;
