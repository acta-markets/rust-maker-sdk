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

    if config.endpoint == MakerWsEndpoint::Vault && subscriptions.wants_vault_feed() {
        let mut recovery = subscriptions.recovery(&SubscriptionsMessage {
            request_id: Uuid::new_v4(),
            channels: Vec::new(),
            underlying_mints: vec![],
            quote_mints: vec![],
        });
        if let Some(subscribe) = &mut recovery.subscribe {
            subscribe.underlying_mints = Vec::new();
            subscribe.quote_mints = Vec::new();
        }
        recover_subscriptions(client, config, inbound, recovery).await?;
    }
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
    let reads = match config.endpoint {
        MakerWsEndpoint::Quote => quote_reads(),
        MakerWsEndpoint::Data => vec![mm_summary_read()],
        MakerWsEndpoint::Vault => {
            let mut reads = vec![
                ClientMessage::GetMyActiveRfqs(GetMyActiveRfqsMessage {
                    request_id: Uuid::new_v4(),
                }),
                ClientMessage::GetPositions(GetPositionsMessage {
                    request_id: Uuid::new_v4(),
                    market: crate::ws::types::QueryFilter::All {},
                    underlying_mint: crate::ws::types::QueryFilter::All {},
                    status: vec![],
                    min_expiry_ts: crate::ws::types::TimeBound::Unbounded {},
                }),
            ];
            if auth
                .vault
                .as_ref()
                .is_some_and(|vault| vault.permissions.can_maker())
            {
                reads.push(mm_summary_read());
                reads.extend(quote_reads());
            }
            reads
        }
    };
    for message in &reads {
        write_message(client, config, message)
            .await
            .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
    }
    wait_for_reconciliation(client, inbound, reads).await
}

fn quote_reads() -> Vec<ClientMessage> {
    vec![
        ClientMessage::GetActiveRfqs(GetActiveRfqsMessage {
            request_id: Uuid::new_v4(),
        }),
        ClientMessage::GetMyQuotes(GetMyQuotesMessage {
            request_id: Uuid::new_v4(),
            scope: MakerQuoteScope::Live {},
        }),
    ]
}

fn mm_summary_read() -> ClientMessage {
    ClientMessage::GetMmSummary(GetMmSummaryMessage {
        request_id: Uuid::new_v4(),
    })
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

    let Some(subscribe) = recovery.subscribe else {
        return Ok(());
    };
    let request_id = subscribe.request_id;
    write_message(client, config, &ClientMessage::Subscribe(subscribe))
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
