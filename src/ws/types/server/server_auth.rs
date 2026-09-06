use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequestData {
    pub challenge: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSuccessData {
    pub session_id: String,
    #[serde_as(as = "UnixSeconds")]
    pub expires_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker_pda: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthErrorData {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl AuthErrorData {
    #[must_use]
    pub fn is_session_expired(&self) -> bool {
        self.reason == "session_expired"
    }

    /// A newer connection authenticated with the same maker identity.
    #[must_use]
    pub fn is_session_replaced(&self) -> bool {
        self.reason == "session_replaced"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogoutSuccessData {}
