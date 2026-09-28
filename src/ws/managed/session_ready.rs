use std::sync::Arc;

use uuid::Uuid;

use super::auth::AuthOutcome;
use super::io::write_message;
use super::{InboundPublisher, ManagedWsConfig, SessionFailure};
use crate::ws::client::WsClient;
use crate::ws::managed::{
    MakerWsEndpoint,
    desired_subscriptions::{DesiredSubscriptions, SubscriptionRecovery},
};
use crate::ws::types::{
    ClientMessage, GetActiveRfqsMessage, GetMmSummaryMessage, GetMyActiveRfqsMessage,
    GetMyQuotesMessage, GetPositionsMessage, GetSubscriptionsMessage, MakerQuoteScope,
    ServerMessage, SubscriptionsMessage,
};

/// Re-establish the desired subscription target and authoritative maker reads
/// before a managed endpoint can report `Ready`.
pub(super) async fn establish(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
    subscriptions: &DesiredSubscriptions,
    auth: &AuthOutcome,
) -> Result<(), SessionFailure> {
    if config.endpoint == MakerWsEndpoint::Quote {
        let actual_request_id = Uuid::new_v4();
        write_message(
            client,
            config,
            &ClientMessage::GetSubscriptions(GetSubscriptionsMessage {
                request_id: actual_request_id,
            }),
        )
        .await
        .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
        let actual = wait_for_subscriptions(client, inbound, actual_request_id).await?;

        let recovery = subscriptions.recovery(&actual);
        recover_subscriptions(client, config, inbound, recovery).await?;
    }

    // The server enables Vaults on every fresh auth. Only an explicit removal
    // needs restoring; no mint scopes apply to the vault endpoint.
    if config.endpoint == MakerWsEndpoint::Vault && !subscriptions.wants_vault_feed() {
        let request_id = Uuid::new_v4();
        write_message(
            client,
            config,
            &ClientMessage::Unsubscribe(crate::ws::types::UnsubscribeData {
                request_id,
                channels: vec![crate::ws::types::WsChannel::Vaults],
            }),
        )
        .await
        .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
        wait_for_unsubscribe_ack(client, inbound, request_id).await?;
    }
    let mut reads = Vec::new();
    if auth.vault.is_some() {
        reads.push(ClientMessage::GetMyActiveRfqs(GetMyActiveRfqsMessage {
            request_id: Uuid::new_v4(),
        }));
        reads.push(ClientMessage::GetPositions(GetPositionsMessage {
            request_id: Uuid::new_v4(),
            market: None,
            underlying_mint: None,
            status: None,
            min_expiry_ts: None,
        }));
    }
    if auth
        .vault
        .as_ref()
        .is_none_or(|vault| vault.permissions.can_maker())
    {
        reads.extend([
            ClientMessage::GetMmSummary(GetMmSummaryMessage {
                request_id: Uuid::new_v4(),
            }),
            ClientMessage::GetActiveRfqs(GetActiveRfqsMessage {
                request_id: Uuid::new_v4(),
            }),
            ClientMessage::GetMyQuotes(GetMyQuotesMessage {
                request_id: Uuid::new_v4(),
                scope: MakerQuoteScope::Live,
                limit: None,
                cursor: None,
                cursor_id: None,
            }),
        ]);
    }
    for message in &reads {
        write_message(client, config, message)
            .await
            .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
    }
    wait_for_reconciliation(client, inbound, reads).await
}

async fn recover_subscriptions(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
    recovery: SubscriptionRecovery,
) -> Result<(), SessionFailure> {
    if let Some(unsubscribe) = recovery.unsubscribe {
        let request_id = unsubscribe.request_id;
        write_message(client, config, &ClientMessage::Unsubscribe(unsubscribe))
            .await
            .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
        wait_for_unsubscribe_ack(client, inbound, request_id).await?;
    }

    let request_id = recovery.subscribe.request_id;
    write_message(
        client,
        config,
        &ClientMessage::Subscribe(recovery.subscribe),
    )
    .await
    .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
    wait_for_subscribe_ack(client, inbound, request_id).await
}

async fn wait_for_subscriptions(
    client: &mut WsClient,
    inbound: &mut InboundPublisher<'_>,
    request_id: Uuid,
) -> Result<SubscriptionsMessage, SessionFailure> {
    loop {
        let message = next_message(client, inbound).await?;
        match &*message {
            ServerMessage::Subscriptions(data) if data.request_id == request_id => {
                return Ok(data.clone());
            }
            ServerMessage::RequestError(error) if error.request_id == request_id => {
                return Err(request_failed(error.request_id, &error.error));
            }
            _ => {}
        }
    }
}

async fn wait_for_subscribe_ack(
    client: &mut WsClient,
    inbound: &mut InboundPublisher<'_>,
    request_id: Uuid,
) -> Result<(), SessionFailure> {
    loop {
        let message = next_message(client, inbound).await?;
        match &*message {
            ServerMessage::SubscribeAck(data) if data.request_id == request_id => return Ok(()),
            ServerMessage::RequestError(error) if error.request_id == request_id => {
                return Err(request_failed(error.request_id, &error.error));
            }
            _ => {}
        }
    }
}

async fn wait_for_unsubscribe_ack(
    client: &mut WsClient,
    inbound: &mut InboundPublisher<'_>,
    request_id: Uuid,
) -> Result<(), SessionFailure> {
    loop {
        let message = next_message(client, inbound).await?;
        match &*message {
            ServerMessage::UnsubscribeAck(data) if data.request_id == request_id => return Ok(()),
            ServerMessage::RequestError(error) if error.request_id == request_id => {
                return Err(request_failed(error.request_id, &error.error));
            }
            _ => {}
        }
    }
}

async fn wait_for_reconciliation(
    client: &mut WsClient,
    inbound: &mut InboundPublisher<'_>,
    mut pending: Vec<ClientMessage>,
) -> Result<(), SessionFailure> {
    while !pending.is_empty() {
        let message = next_message(client, inbound).await?;
        if let ServerMessage::RequestError(error) = &*message
            && pending
                .iter()
                .any(|request| request.request_id() == Some(error.request_id))
        {
            return Err(request_failed(error.request_id, &error.error));
        }
        let index = pending
            .iter()
            .position(|request| match (request, &*message) {
                (ClientMessage::GetMmSummary(request), ServerMessage::MmSummary(data)) => {
                    request.request_id == data.request_id
                }
                (ClientMessage::GetActiveRfqs(request), ServerMessage::ActiveRfqs(data)) => {
                    request.request_id == data.request_id
                }
                (ClientMessage::GetMyQuotes(request), ServerMessage::MyQuotes(data)) => {
                    request.request_id == data.request_id
                }
                (ClientMessage::GetMyActiveRfqs(request), ServerMessage::MyActiveRfqs(data)) => {
                    request.request_id == data.request_id
                }
                (ClientMessage::GetPositions(request), ServerMessage::Positions(data)) => {
                    request.request_id == data.request_id
                }
                _ => false,
            });
        if let Some(index) = index {
            if let ServerMessage::MyQuotes(data) = &*message
                && data.has_more
            {
                return Err(SessionFailure::Retryable(format!(
                    "readiness Live quote snapshot {} was truncated",
                    data.request_id
                )));
            }
            pending.remove(index);
        }
    }
    Ok(())
}

async fn next_message(
    client: &mut WsClient,
    inbound: &mut InboundPublisher<'_>,
) -> Result<Arc<ServerMessage>, SessionFailure> {
    let message = match client.next().await {
        Some(Ok(message)) => message,
        Some(Err(error)) => return Err(SessionFailure::Retryable(error.to_string())),
        None => {
            return Err(SessionFailure::Retryable(
                "connection closed during readiness barrier".to_string(),
            ));
        }
    };
    let message = inbound.publish(message);
    if let Some(failure) = SessionFailure::from_control(&message) {
        return Err(failure);
    }
    Ok(message)
}

fn request_failed(request_id: Uuid, error: &impl std::fmt::Debug) -> SessionFailure {
    SessionFailure::Retryable(format!("readiness request {request_id} failed: {error:?}"))
}
