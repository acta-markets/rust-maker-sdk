use crate::ws::types::{AuthErrorData, ServerError, ServerMessage, WsErrorCode};

use super::ManagedWsTerminationReason;

/// A control frame that ends the current session attempt.
pub(super) enum ControlFailure {
    Retry(String),
    ClearCredentials(String),
    Terminate(ManagedWsTerminationReason),
}

pub(super) fn classify(message: &ServerMessage) -> Option<ControlFailure> {
    match message {
        ServerMessage::VersionMismatch(data) => Some(ControlFailure::Terminate(
            ManagedWsTerminationReason::ProtocolVersionMismatch {
                requested_version: data.requested_version.clone(),
                server_version: data.server_version.clone(),
                min_supported_version: data.min_supported_version.clone(),
            },
        )),
        ServerMessage::AuthError(error) => Some(classify_auth_error(error)),
        ServerMessage::Error(ServerError::Generic { code, message }) => {
            classify_code(code, Some(message.as_str()))
        }
        ServerMessage::Error(ServerError::ServerShuttingDown) => {
            Some(ControlFailure::Retry("server is shutting down".to_string()))
        }
        _ => None,
    }
}

/// Every `AuthError` ends the attempt; unknown reasons trigger a retry.
pub(super) fn classify_auth_error(error: &AuthErrorData) -> ControlFailure {
    let code = &error.reason;
    let message = Some(error.message.as_str());
    match classify_code(code, message) {
        Some(failure) => failure,
        None => ControlFailure::Retry(format_detail(code, message)),
    }
}

fn classify_code(code: &WsErrorCode, message: Option<&str>) -> Option<ControlFailure> {
    match code {
        WsErrorCode::SessionReplaced => Some(ControlFailure::Terminate(
            ManagedWsTerminationReason::SessionReplaced,
        )),
        WsErrorCode::UnsupportedProtocolVersion => Some(ControlFailure::Terminate(
            ManagedWsTerminationReason::UnsupportedProtocolVersion,
        )),
        WsErrorCode::Blacklisted
        | WsErrorCode::MakerUnregistered
        | WsErrorCode::MakerNotRegistered
        | WsErrorCode::InvalidSignature
        | WsErrorCode::OperatorMismatch
        | WsErrorCode::VaultOperatorUnauthorized => Some(ControlFailure::Terminate(
            ManagedWsTerminationReason::AuthenticationRejected {
                reason: code.to_string(),
                message: message.map(str::to_owned),
            },
        )),
        WsErrorCode::VaultSessionInvalidated => {
            Some(ControlFailure::Retry(format_detail(code, message)))
        }
        WsErrorCode::SessionExpired => Some(ControlFailure::ClearCredentials(format_detail(
            code, message,
        ))),
        _ => None,
    }
}

fn format_detail(code: &WsErrorCode, message: Option<&str>) -> String {
    match message {
        Some(message) => format!("{code} ({message})"),
        None => code.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlFailure, classify};
    use crate::ws::managed::ManagedWsTerminationReason;
    use crate::ws::types::{ServerError, ServerMessage, VersionMismatchData, WsErrorCode};

    #[test]
    fn backend_session_replaced_frame_is_terminal() {
        let action = classify(&ServerMessage::Error(ServerError::Generic {
            code: WsErrorCode::SessionReplaced,
            message: "This session was replaced by a newer connection".to_string(),
        }));

        assert!(matches!(
            action,
            Some(ControlFailure::Terminate(
                ManagedWsTerminationReason::SessionReplaced
            ))
        ));
    }

    #[test]
    fn version_mismatch_is_terminal() {
        let action = classify(&ServerMessage::VersionMismatch(VersionMismatchData {
            requested_version: "0.9.0".to_string(),
            server_version: "1.1.0".to_string(),
            min_supported_version: crate::WS_PROTOCOL_VERSION.to_string(),
            message: "unsupported".to_string(),
        }));

        assert!(matches!(
            action,
            Some(ControlFailure::Terminate(
                ManagedWsTerminationReason::ProtocolVersionMismatch { .. }
            ))
        ));
    }

    #[test]
    fn expired_session_clears_resume_credentials() {
        let action = classify(&ServerMessage::Error(ServerError::Generic {
            code: WsErrorCode::SessionExpired,
            message: "Authentication session expired".to_string(),
        }));

        assert!(matches!(action, Some(ControlFailure::ClearCredentials(_))));
    }

    #[test]
    fn unsupported_protocol_is_a_typed_terminal_outcome() {
        let action = classify(&ServerMessage::Error(ServerError::Generic {
            code: WsErrorCode::UnsupportedProtocolVersion,
            message: "spreads".to_string(),
        }));

        assert!(matches!(
            action,
            Some(ControlFailure::Terminate(
                ManagedWsTerminationReason::UnsupportedProtocolVersion
            ))
        ));
    }
}
