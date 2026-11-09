use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use uuid::Uuid;

use super::auth::AuthOutcome;
use super::io::write_message;
use super::{InboundPublisher, ManagedWsConfig, SessionFailure};
use crate::ws::client::WsClient;
use crate::ws::managed::{
    MakerWsEndpoint,
    desired_subscriptions::{DesiredSubscriptions, SubscriptionRecovery},
};
use crate::ws::types::*;

/// The step `establish` is on, for a failure or timeout report.
#[derive(Debug, Default)]
pub(super) struct ReadinessStep(AtomicU8);

impl ReadinessStep {
    const NAMES: [&'static str; 3] = [
        "subscription snapshot",
        "subscription recovery",
        "reconciliation reads",
    ];

    fn enter(&self, index: u8) {
        self.0.store(index, Ordering::Relaxed);
    }

    pub(super) fn name(&self) -> &'static str {
        Self::NAMES[usize::from(self.0.load(Ordering::Relaxed))]
    }
}

/// Re-establish the desired subscription target and authoritative maker reads
/// before a managed endpoint can report `Ready`.
pub(super) async fn establish(
    client: &mut WsClient,
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
    subscriptions: &DesiredSubscriptions,
    auth: &AuthOutcome,
    step: &ReadinessStep,
) -> Result<(), SessionFailure> {
    if config.endpoint == MakerWsEndpoint::Quote {
        let mut request = GetSubscriptionsMessage::default();
        let mut actual = SubscriptionsMessage {
            request_id: request.request_id,
            next_cursor: None,
            channels: Vec::new(),
            underlying_mints: Vec::new(),
            quote_mints: Vec::new(),
        };
        let mut continuation = crate::paging::Continuation::default();
        loop {
            write_message(
                client,
                config,
                &ClientMessage::GetSubscriptions(request.clone()),
            )
            .await
            .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
            let page = wait_for_subscriptions(client, inbound, request.request_id).await?;
            use crate::ws::maker::PagedRequest;
            GetSubscriptionsMessage::append(&mut actual, page);
            let Some(cursor) = continuation
                .advance(actual.next_cursor.as_deref())
                .map_err(|error| SessionFailure::Retryable(error.to_string()))?
            else {
                break;
            };
            request.cursor = cursor;
            request.request_id = Uuid::new_v4();
        }

        step.enter(1);
        let recovery = subscriptions.recovery(&actual);
        recover_subscriptions(client, config, inbound, recovery).await?;
    }

    if config.endpoint == MakerWsEndpoint::Vault {
        step.enter(1);
    }
    if config.endpoint == MakerWsEndpoint::Vault && subscriptions.wants_vault_feed() {
        let mut recovery = subscriptions.recovery(&SubscriptionsMessage {
            request_id: Uuid::new_v4(),
            next_cursor: None,
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
    step.enter(2);
    let reads = match config.endpoint {
        MakerWsEndpoint::Quote => quote_reads(),
        MakerWsEndpoint::Data => vec![mm_summary_read()],
        MakerWsEndpoint::Vault => {
            let mut reads = vec![
                ClientMessage::GetMyActiveRfqs(GetMyActiveRfqsMessage {
                    limit: crate::paging::MAX_PAGE_SIZE,
                    cursor: Cursor::Start {},

                    request_id: Uuid::new_v4(),
                }),
                ClientMessage::GetPositions(GetPositionsMessage {
                    limit: crate::paging::MAX_PAGE_SIZE,
                    cursor: Cursor::Start {},

                    request_id: Uuid::new_v4(),
                    market: crate::ws::types::QueryFilter::All {},
                    underlying_mint: crate::ws::types::QueryFilter::All {},
                    status: vec![
                        crate::ws::types::PositionStatus::Open,
                        crate::ws::types::PositionStatus::Funded,
                    ],
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
    wait_for_reconciliation(client, config, inbound, reads).await
}

fn quote_reads() -> Vec<ClientMessage> {
    vec![
        ClientMessage::GetActiveRfqs(GetActiveRfqsMessage {
            limit: crate::paging::MAX_PAGE_SIZE,
            cursor: Cursor::Start {},

            request_id: Uuid::new_v4(),
        }),
        ClientMessage::GetMyQuotes(GetMyQuotesMessage {
            request_id: Uuid::new_v4(),
            scope: MakerQuoteScope::Live {},
            limit: crate::paging::MAX_PAGE_SIZE,
            cursor: Cursor::Start {},
        }),
    ]
}

fn mm_summary_read() -> ClientMessage {
    ClientMessage::GetMmSummary(GetMmSummaryMessage {
        limit: crate::paging::MAX_PAGE_SIZE,
        cursor: Cursor::Start {},

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
    config: &ManagedWsConfig,
    inbound: &mut InboundPublisher<'_>,
    mut pending: Vec<ClientMessage>,
) -> Result<(), SessionFailure> {
    let mut continuations = std::collections::HashMap::<&str, crate::paging::Continuation>::new();
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
                (
                    ClientMessage::GetMakerPositions(request),
                    ServerMessage::MakerPositions(data),
                ) => request.request_id == data.request_id,
                (ClientMessage::GetPositions(request), ServerMessage::Positions(data)) => {
                    request.request_id == data.request_id
                }
                (ClientMessage::GetMyCaps(request), ServerMessage::MyCaps(data)) => {
                    request.request_id == data.request_id
                }
                (ClientMessage::GetMarketsForMaker(request), ServerMessage::MakerMarkets(data)) => {
                    request.request_id == data.request_id
                }
                (ClientMessage::GetTokens(request), ServerMessage::Tokens(data)) => {
                    request.request_id == data.request_id
                }
                _ => false,
            });
        if let Some(index) = index {
            let request = pending.remove(index);
            for next in reconciliation_pages(request, &message, &mut continuations)? {
                // Vault recovery already starts this live quote stream alongside
                // its summary; only the data endpoint needs this follow-up.
                if matches!(next, ClientMessage::GetMyQuotes(_))
                    && matches!(message.as_ref(), ServerMessage::MmSummary(_))
                    && config.endpoint != MakerWsEndpoint::Data
                {
                    continue;
                }
                write_message(client, config, &next)
                    .await
                    .map_err(|error| SessionFailure::Retryable(error.to_string()))?;
                pending.push(next);
            }
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

fn reconciliation_pages(
    request: ClientMessage,
    response: &ServerMessage,
    continuations: &mut std::collections::HashMap<&'static str, crate::paging::Continuation>,
) -> Result<Vec<ClientMessage>, SessionFailure> {
    let mut pages = Vec::new();
    macro_rules! continue_page {
        ($request:ident, $data:ident, $variant:ident) => {
            if let Some(cursor) = continuations
                .entry(stringify!($variant))
                .or_default()
                .advance($data.next_cursor.as_deref())
                .map_err(|error| SessionFailure::Retryable(error.to_string()))?
            {
                $request.request_id = Uuid::new_v4();
                $request.cursor = cursor;
                pages.push(ClientMessage::$variant($request));
            }
        };
    }
    match (request, response) {
        (ClientMessage::GetPositions(mut request), ServerMessage::Positions(data)) => {
            continue_page!(request, data, GetPositions)
        }
        (ClientMessage::GetActiveRfqs(mut request), ServerMessage::ActiveRfqs(data)) => {
            continue_page!(request, data, GetActiveRfqs)
        }
        (ClientMessage::GetMyActiveRfqs(mut request), ServerMessage::MyActiveRfqs(data)) => {
            continue_page!(request, data, GetMyActiveRfqs)
        }
        (ClientMessage::GetMyQuotes(mut request), ServerMessage::MyQuotes(data)) => {
            continue_page!(request, data, GetMyQuotes)
        }
        (ClientMessage::GetMakerPositions(mut request), ServerMessage::MakerPositions(data)) => {
            continue_page!(request, data, GetMakerPositions)
        }
        (ClientMessage::GetMyCaps(mut request), ServerMessage::MyCaps(data)) => {
            continue_page!(request, data, GetMyCaps)
        }
        (ClientMessage::GetMarketsForMaker(mut request), ServerMessage::MakerMarkets(data)) => {
            continue_page!(request, data, GetMarketsForMaker)
        }
        (ClientMessage::GetTokens(mut request), ServerMessage::Tokens(data)) => {
            continue_page!(request, data, GetTokens)
        }
        (ClientMessage::GetMmSummary(_), ServerMessage::MmSummary(data))
            if data.next_cursor.is_some() =>
        {
            // Recovery needs all live rows and reference/cap collections. It must
            // not traverse the wallet's terminal history just to reach those groups.
            pages.push(ClientMessage::GetMakerPositions(GetMakerPositionsMessage {
                request_id: Uuid::new_v4(),
                market: QueryFilter::All {},
                underlying_mint: QueryFilter::All {},
                status: vec![PositionStatus::Open, PositionStatus::Funded],
                min_expiry_ts: TimeBound::Unbounded {},
                limit: crate::paging::MAX_PAGE_SIZE,
                cursor: Cursor::Start {},
            }));
            pages.push(ClientMessage::GetMyQuotes(GetMyQuotesMessage::default()));
            pages.push(ClientMessage::GetMyCaps(GetMyCapsMessage {
                request_id: Uuid::new_v4(),
                limit: crate::paging::MAX_PAGE_SIZE,
                cursor: Cursor::Start {},
            }));
            pages.push(ClientMessage::GetMarketsForMaker(
                GetMarketsForMakerMessage::default(),
            ));
            pages.push(ClientMessage::GetTokens(GetTokensMessage {
                request_id: Uuid::new_v4(),
                limit: crate::paging::MAX_PAGE_SIZE,
                cursor: Cursor::Start {},
            }));
        }
        _ => {}
    }
    Ok(pages)
}

#[cfg(test)]
mod step_tests {
    use super::ReadinessStep;

    #[test]
    fn step_names_follow_progress() {
        let step = ReadinessStep::default();
        assert_eq!(step.name(), "subscription snapshot");
        step.enter(2);
        assert_eq!(step.name(), "reconciliation reads");
    }
}
