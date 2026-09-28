use tokio::time::timeout;

use super::control::{self, ControlFailure};
use super::{InboundPublisher, ManagedWsConfig, ManagedWsTerminationReason};
use crate::signing::SigningError;
use crate::ws::client::WsClient;
use crate::ws::error::WsClientError;
use crate::ws::types::{
    AuthChallengeData, ClientMessage, ResumeAuthData, ServerMessage, StartAuthData,
    StartVaultAuthData, VaultAuthSuccessData,
};

/// What the server granted. `vault` is set only by the vault flow.
pub(super) struct AuthOutcome {
    pub(super) vault: Option<VaultAuthSuccessData>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum AuthenticationError {
    #[error(transparent)]
    Transport(#[from] WsClientError),
    #[error("authentication rejected: {detail}")]
    Rejected { detail: String },
    #[error("authentication terminated: {0:?}")]
    Terminated(ManagedWsTerminationReason),
    #[error("challenge signer failed: {0}")]
    Signer(SigningError),
}

impl AuthenticationError {
    pub(super) fn termination_reason(&self) -> Option<ManagedWsTerminationReason> {
        match self {
            Self::Terminated(reason) => Some(reason.clone()),
            _ => None,
        }
    }
}

pub(super) async fn build_auth_response(
    config: &ManagedWsConfig,
    challenge: &str,
) -> Result<AuthChallengeData, SigningError> {
    if let Some(vault) = config.vault_pda() {
        crate::ws::auth::validate_vault_auth_challenge(challenge, &config.auth_pubkey, vault)?;
    } else {
        crate::ws::auth::validate_maker_auth_challenge(challenge)?;
    }
    let signature = config.challenge_signing.sign(challenge).await?;
    Ok(AuthChallengeData {
        challenge: challenge.to_owned(),
        signature,
        pubkey: config.auth_pubkey.clone(),
    })
}

pub(super) async fn authenticate(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
    resume_session_id: &mut Option<String>,
) -> Result<AuthOutcome, AuthenticationError> {
    let hello = ClientMessage::Hello(config.hello());
    timeout(config.write_timeout, client.send(&hello))
        .await
        .map_err(|_| WsClientError::Timeout)??;

    // Wait for protocol negotiation before starting authentication.
    loop {
        let message = match client.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(err)) => return Err(err.into()),
            None => return Err(WsClientError::ConnectionClosed.into()),
        };
        let msg_arc = inbound.publish(message);
        match &*msg_arc {
            ServerMessage::Welcome(welcome) => {
                if let Some(feature) = config.missing_required_feature(&welcome.enabled_features) {
                    return Err(AuthenticationError::Terminated(
                        ManagedWsTerminationReason::FeatureUnsupported {
                            feature: feature.to_owned(),
                        },
                    ));
                }
                break;
            }
            message => {
                if let Some(failure) = control::classify(message) {
                    return control_error(failure, resume_session_id);
                }
            }
        }
    }

    if let Some(vault_pda) = config.vault_pda() {
        return authenticate_vault(client, config, inbound, resume_session_id, vault_pda).await;
    }

    let mut pending_challenge = None;
    if let Some(session_id) = resume_session_id.clone() {
        timeout(
            config.write_timeout,
            client.resume_auth(ResumeAuthData { session_id }),
        )
        .await
        .map_err(|_| WsClientError::Timeout)??;

        loop {
            let message = match client.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(err)) => return Err(err.into()),
                None => return Err(WsClientError::ConnectionClosed.into()),
            };
            let msg_arc = inbound.publish(message);
            match &*msg_arc {
                ServerMessage::AuthSuccess(data) => {
                    *resume_session_id = Some(data.session_id.clone());
                    return Ok(AuthOutcome { vault: None });
                }
                ServerMessage::AuthError(err) if err.is_session_expired() => {
                    *resume_session_id = None;
                    break;
                }
                ServerMessage::AuthError(err) => {
                    return control_error(control::classify_auth_error(err), resume_session_id);
                }
                ServerMessage::AuthRequest(data) => {
                    pending_challenge = Some(data.challenge.clone());
                }
                message => {
                    if let Some(failure) = control::classify(message) {
                        return control_error(failure, resume_session_id);
                    }
                }
            }
        }
    }

    if let Some(challenge) = pending_challenge {
        send_auth_response(client, config, &challenge).await?;
    } else {
        let start_auth = ClientMessage::StartAuth(StartAuthData {
            pubkey: config.auth_pubkey.clone(),
        });
        timeout(config.write_timeout, client.send(&start_auth))
            .await
            .map_err(|_| WsClientError::Timeout)??;
    }

    loop {
        let message = match client.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(err)) => return Err(err.into()),
            None => return Err(WsClientError::ConnectionClosed.into()),
        };
        let msg_arc = inbound.publish(message);
        match &*msg_arc {
            ServerMessage::AuthRequest(data) => {
                send_auth_response(client, config, &data.challenge).await?;
            }
            ServerMessage::AuthSuccess(data) => {
                *resume_session_id = Some(data.session_id.clone());
                return Ok(AuthOutcome { vault: None });
            }
            ServerMessage::AuthError(err) => {
                return control_error(control::classify_auth_error(err), resume_session_id);
            }
            message => {
                if let Some(failure) = control::classify(message) {
                    return control_error(failure, resume_session_id);
                }
            }
        }
    }
}

/// `StartVaultAuth` -> `AuthRequest` -> `AuthChallenge` -> `VaultAuthSuccess`.
/// The server issues no session id here, so nothing is stored for resume.
async fn authenticate_vault(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
    resume_session_id: &mut Option<String>,
    vault_pda: &str,
) -> Result<AuthOutcome, AuthenticationError> {
    let start = ClientMessage::StartVaultAuth(StartVaultAuthData {
        operator: config.auth_pubkey.clone(),
        vault_pda: vault_pda.to_owned(),
    });
    timeout(config.write_timeout, client.send(&start))
        .await
        .map_err(|_| WsClientError::Timeout)??;

    loop {
        let message = match client.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(err)) => return Err(err.into()),
            None => return Err(WsClientError::ConnectionClosed.into()),
        };
        let msg_arc = inbound.publish(message);
        match &*msg_arc {
            ServerMessage::AuthRequest(data) => {
                send_auth_response(client, config, &data.challenge).await?;
            }
            ServerMessage::VaultAuthSuccess(data) => {
                return Ok(AuthOutcome {
                    vault: Some(data.clone()),
                });
            }
            ServerMessage::AuthSuccess(_) => {
                return Err(AuthenticationError::Rejected {
                    detail: "server answered vault auth with a maker session".to_owned(),
                });
            }
            ServerMessage::AuthError(err) => {
                return control_error(control::classify_auth_error(err), resume_session_id);
            }
            message => {
                if let Some(failure) = control::classify(message) {
                    return control_error(failure, resume_session_id);
                }
            }
        }
    }
}

async fn send_auth_response(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    challenge: &str,
) -> Result<(), AuthenticationError> {
    let auth = build_auth_response(config, challenge)
        .await
        .map_err(AuthenticationError::Signer)?;
    timeout(config.write_timeout, client.auth_challenge(auth))
        .await
        .map_err(|_| WsClientError::Timeout)??;
    Ok(())
}

fn control_error<T>(
    failure: ControlFailure,
    resume_session_id: &mut Option<String>,
) -> Result<T, AuthenticationError> {
    match failure {
        ControlFailure::Retry(detail) => Err(AuthenticationError::Rejected { detail }),
        ControlFailure::ClearCredentials(detail) => {
            *resume_session_id = None;
            Err(AuthenticationError::Rejected { detail })
        }
        ControlFailure::Terminate(reason) => Err(AuthenticationError::Terminated(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::AuthenticationError;
    use crate::signing::SigningError;

    #[test]
    fn rejected_auth_preserves_server_detail() {
        let error = AuthenticationError::Rejected {
            detail: "invalid_signature (nonce expired)".to_string(),
        };

        assert_eq!(
            error.to_string(),
            "authentication rejected: invalid_signature (nonce expired)"
        );
    }

    #[test]
    fn signer_error_has_one_context_prefix() {
        let error = AuthenticationError::Signer(SigningError::new("hardware signer offline"));

        assert_eq!(
            error.to_string(),
            "challenge signer failed: hardware signer offline"
        );
    }
}
