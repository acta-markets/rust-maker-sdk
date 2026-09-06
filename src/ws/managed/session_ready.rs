use std::sync::Arc;

use uuid::Uuid;

use super::io::write_message;
use super::{InboundPublisher, ManagedWsConfig, SessionFailure};
use crate::ws::client::WsClient;
use crate::ws::managed::{
    MakerWsEndpoint,
    desired_subscriptions::{DesiredSubscriptions, SubscriptionRecovery},
};
use crate::ws::types::{
    ClientMessage, GetActiveRfqsMessage, GetMmSummaryMessage, GetMyQuotesMessage,
    GetSubscriptionsMessage, MakerQuoteScope, ServerMessage, SubscriptionsMessage,
};

/// Re-establish the desired subscription target and authoritative maker reads
/// before a managed endpoint can report `Ready`.
pub(super) async fn establish(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
    subscriptions: &DesiredSubscriptions,
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

    let mm_summary_id = Uuid::new_v4();
    let active_rfqs_id = Uuid::new_v4();
    let my_quotes_id = Uuid::new_v4();
    for message in [
        ClientMessage::GetMmSummary(GetMmSummaryMessage {
            request_id: mm_summary_id,
        }),
        ClientMessage::GetActiveRfqs(GetActiveRfqsMessage {
            request_id: active_rfqs_id,
        }),
        ClientMessage::GetMyQuotes(GetMyQuotesMessage {
            request_id: my_quotes_id,
            scope: MakerQuoteScope::Live,
            limit: None,
            cursor: None,
            cursor_id: None,
        }),
    ] {
        write_message(client, config, &message)
            .await
            .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
    }
    wait_for_reconciliation(client, inbound, mm_summary_id, active_rfqs_id, my_quotes_id).await
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
    mm_summary_id: Uuid,
    active_rfqs_id: Uuid,
    my_quotes_id: Uuid,
) -> Result<(), SessionFailure> {
    let mut mm_summary_ready = false;
    let mut active_rfqs_ready = false;
    let mut my_quotes_ready = false;
    while !(mm_summary_ready && active_rfqs_ready && my_quotes_ready) {
        let message = next_message(client, inbound).await?;
        match &*message {
            ServerMessage::MmSummary(data) if data.request_id == mm_summary_id => {
                mm_summary_ready = true;
            }
            ServerMessage::ActiveRfqs(data) if data.request_id == active_rfqs_id => {
                active_rfqs_ready = true;
            }
            ServerMessage::MyQuotes(data) if data.request_id == my_quotes_id => {
                if data.has_more {
                    return Err(SessionFailure::Retryable(format!(
                        "readiness Live quote snapshot {} was truncated",
                        data.request_id
                    )));
                }
                my_quotes_ready = true;
            }
            ServerMessage::RequestError(error)
                if [mm_summary_id, active_rfqs_id, my_quotes_id].contains(&error.request_id) =>
            {
                return Err(request_failed(error.request_id, &error.error));
            }
            _ => {}
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
