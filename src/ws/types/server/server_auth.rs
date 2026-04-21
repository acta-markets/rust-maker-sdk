use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, serde_as};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequestData {
    pub challenge: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSuccessData {
    pub session_id: String,
    #[serde_as(as = "Option<TimestampSeconds<i64>>")]
    pub expires_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker_pda: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthErrorData {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogoutSuccessData {}
