use crate::Address32;
use crate::types::unix_time::UnixSeconds;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use super::super::error_code::WsErrorCode;

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
    pub role: AuthRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthRole {
    Taker,
    Maker { maker_pda: Address32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthErrorData {
    pub reason: WsErrorCode,
    pub message: String,
}

impl AuthErrorData {
    #[must_use]
    pub fn is_session_expired(&self) -> bool {
        matches!(self.reason, WsErrorCode::SessionExpired)
    }

    /// A newer connection authenticated with the same maker identity.
    #[must_use]
    pub fn is_session_replaced(&self) -> bool {
        matches!(self.reason, WsErrorCode::SessionReplaced)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogoutSuccessData {}
