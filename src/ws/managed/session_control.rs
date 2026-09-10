use crate::ws::types::{AuthErrorData, ServerError, ServerMessage};

use super::ManagedWsTerminationReason;

/// A control failure that ends the current session attempt.
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

/// An `AuthError` frame always ends the attempt, so this is total: an
/// unrecognised reason is retryable rather than "keep going".
pub(super) fn classify_auth_error(error: &AuthErrorData) -> ControlFailure {
    let code = error.reason.as_str();
    let message = error.message.as_deref();
    match classify_code(code, message) {
        Some(failure) => failure,
        None => ControlFailure::Retry(format_detail(code, message)),
    }
}

fn classify_code(code: &str, message: Option<&str>) -> Option<ControlFailure> {
    match code {
        "session_replaced" => Some(ControlFailure::Terminate(
            ManagedWsTerminationReason::SessionReplaced,
        )),
        "blacklisted"
        | "maker_unregistered"
        | "maker_not_registered"
        | "invalid_pubkey"
        | "invalid_signature" => Some(ControlFailure::Terminate(
            ManagedWsTerminationReason::AuthenticationRejected {
                reason: code.to_string(),
                message: message.map(str::to_owned),
            },
        )),
        "session_expired" => Some(ControlFailure::ClearCredentials(format_detail(
            code, message,
        ))),
        _ => None,
    }
}

fn format_detail(code: &str, message: Option<&str>) -> String {
    match message {
        Some(message) => format!("{code} ({message})"),
        None => code.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlFailure, classify};
    use crate::ws::managed::ManagedWsTerminationReason;
    use crate::ws::types::{ServerError, ServerMessage, VersionMismatchData};

    #[test]
    fn backend_session_replaced_frame_is_terminal() {
        let action = classify(&ServerMessage::Error(ServerError::Generic {
            code: "session_replaced".to_string(),
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
            min_supported_version: "1.0.0".to_string(),
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
            code: "session_expired".to_string(),
            message: "Authentication session expired".to_string(),
        }));

        assert!(matches!(action, Some(ControlFailure::ClearCredentials(_))));
    }
}
