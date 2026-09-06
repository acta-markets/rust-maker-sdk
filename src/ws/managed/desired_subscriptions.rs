use std::collections::HashSet;

use uuid::Uuid;

use crate::ws::types::{
    AddChannelsData, AddMintsData, RemoveMintsData, ServerMessage, SubscribeData,
    SubscriptionChange, SubscriptionsMessage, UnsubscribeData, WsChannel,
};

/// The local target that a managed quote session re-establishes after every
/// authentication. It records one in-flight mutation so an explicit server
/// rejection can restore the immediately previous target.
#[derive(Debug, Default)]
pub(super) struct DesiredSubscriptions {
    current: SubscriptionSet,
    pending: Option<PendingSubscriptionChange>,
}

#[derive(Debug, Clone, Default)]
struct SubscriptionSet {
    channels: HashSet<WsChannel>,
    underlying_mints: HashSet<String>,
    quote_mints: HashSet<String>,
}

#[derive(Debug)]
struct PendingSubscriptionChange {
    request_id: Uuid,
    previous: SubscriptionSet,
}

pub(super) struct SubscriptionRecovery {
    pub(super) unsubscribe: Option<UnsubscribeData>,
    pub(super) subscribe: SubscribeData,
}

impl DesiredSubscriptions {
    pub(super) fn from_initial(initial: Option<&SubscribeData>) -> Self {
        let mut current = SubscriptionSet::default();
        if let Some(initial) = initial {
            current.apply_subscribe(initial);
        }
        Self {
            current,
            pending: None,
        }
    }

    #[must_use]
    pub(super) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Records a subscription mutation only after it entered the writer lane.
    /// The session actor serializes these calls while `pending` is populated.
    pub(super) fn begin(&mut self, change: &SubscriptionChange) {
        let previous = self.current.clone();
        match change {
            SubscriptionChange::Subscribe(data) => self.current.apply_subscribe(data),
            SubscriptionChange::Unsubscribe(data) => self.current.remove_channels(&data.channels),
            SubscriptionChange::AddMints(data) => self.current.add_mints(data),
            SubscriptionChange::RemoveMints(data) => self.current.remove_mints(data),
            SubscriptionChange::AddChannels(data) => self.current.add_channels(data),
            SubscriptionChange::RemoveChannels(data) => {
                self.current.remove_channels(&data.channels)
            }
        }
        self.pending = Some(PendingSubscriptionChange {
            request_id: change.request_id(),
            previous,
        });
    }

    /// Resolves only the active mutation. A matching request error restores
    /// the prior desired target; all successful subscription replies commit it.
    pub(super) fn resolve(&mut self, message: &ServerMessage) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let request_id = match message {
            ServerMessage::SubscribeAck(data) => data.request_id,
            ServerMessage::UnsubscribeAck(data) => data.request_id,
            ServerMessage::SubscriptionUpdated(data) => data.request_id,
            ServerMessage::RequestError(data) => data.request_id,
            _ => return,
        };
        if pending.request_id != request_id {
            return;
        }

        if matches!(message, ServerMessage::RequestError(_)) {
            self.reject_pending();
        } else {
            self.pending = None;
        }
    }

    fn reject_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.current = pending.previous;
        }
    }

    /// A disconnect makes the command outcome unknown. Keep its desired
    /// target for reconnect recovery, but do not wait for an old ACK.
    pub(super) fn connection_lost(&mut self) {
        self.pending = None;
    }

    /// Build the canonical recovery sequence from the server's current state.
    /// `Subscribe` always carries both mint scopes, including explicit empty
    /// lists, so it replaces stale scopes rather than replaying mutations.
    #[must_use]
    pub(super) fn recovery(&self, actual: &SubscriptionsMessage) -> SubscriptionRecovery {
        let extra_channels = actual
            .channels
            .iter()
            .filter(|channel| !self.current.channels.contains(channel))
            .copied()
            .collect::<Vec<_>>();
        let unsubscribe = (!extra_channels.is_empty()).then(|| UnsubscribeData {
            request_id: Uuid::new_v4(),
            channels: extra_channels,
        });
        SubscriptionRecovery {
            unsubscribe,
            subscribe: SubscribeData {
                request_id: Uuid::new_v4(),
                channels: self.current.channels.iter().copied().collect(),
                underlying_mints: Some(self.current.underlying_mints.iter().cloned().collect()),
                quote_mints: Some(self.current.quote_mints.iter().cloned().collect()),
            },
        }
    }
}

impl SubscriptionSet {
    fn apply_subscribe(&mut self, data: &SubscribeData) {
        self.channels.extend(data.channels.iter().copied());
        if data.underlying_mints.is_some() || data.quote_mints.is_some() {
            self.underlying_mints = normalized(data.underlying_mints.as_deref());
            self.quote_mints = normalized(data.quote_mints.as_deref());
        }
    }

    fn add_mints(&mut self, data: &AddMintsData) {
        self.underlying_mints
            .extend(normalized(data.underlying_mints.as_deref()));
        self.quote_mints
            .extend(normalized(data.quote_mints.as_deref()));
    }

    fn remove_mints(&mut self, data: &RemoveMintsData) {
        for mint in normalized(data.underlying_mints.as_deref()) {
            self.underlying_mints.remove(&mint);
        }
        for mint in normalized(data.quote_mints.as_deref()) {
            self.quote_mints.remove(&mint);
        }
    }

    fn add_channels(&mut self, data: &AddChannelsData) {
        self.channels.extend(data.channels.iter().copied());
    }

    fn remove_channels(&mut self, channels: &[WsChannel]) {
        for channel in channels {
            self.channels.remove(channel);
        }
    }
}

fn normalized(mints: Option<&[String]>) -> HashSet<String> {
    mints
        .into_iter()
        .flatten()
        .map(|mint| mint.trim())
        .filter(|mint| !mint.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::types::{RequestErrorEnvelope, ServerError};

    fn initial() -> SubscribeData {
        SubscribeData {
            request_id: Uuid::new_v4(),
            channels: vec![WsChannel::Rfqs],
            underlying_mints: Some(vec!["underlying".to_string()]),
            quote_mints: Some(vec!["quote".to_string()]),
        }
    }

    #[test]
    fn rejected_mutation_restores_previous_desired_target() {
        let mut desired = DesiredSubscriptions::from_initial(Some(&initial()));
        let request_id = Uuid::new_v4();
        desired.begin(&SubscriptionChange::AddChannels(AddChannelsData {
            request_id,
            channels: vec![WsChannel::Trades],
        }));
        assert!(desired.has_pending());

        desired.resolve(&ServerMessage::RequestError(RequestErrorEnvelope {
            request_id,
            error: ServerError::Generic {
                code: "rejected".to_string(),
                message: "test".to_string(),
            },
        }));

        let recovery = desired.recovery(&SubscriptionsMessage {
            request_id: Uuid::new_v4(),
            channels: Vec::new(),
            underlying_mints: None,
            quote_mints: None,
        });
        assert!(!desired.has_pending());
        assert_eq!(recovery.subscribe.channels, vec![WsChannel::Rfqs]);
    }

    #[test]
    fn disconnect_keeps_accepted_target_for_recovery() {
        let mut desired = DesiredSubscriptions::from_initial(Some(&initial()));
        desired.begin(&SubscriptionChange::RemoveMints(RemoveMintsData {
            request_id: Uuid::new_v4(),
            underlying_mints: Some(vec!["underlying".to_string()]),
            quote_mints: Some(vec!["quote".to_string()]),
        }));
        desired.connection_lost();

        let recovery = desired.recovery(&SubscriptionsMessage {
            request_id: Uuid::new_v4(),
            channels: Vec::new(),
            underlying_mints: Some(vec!["stale".to_string()]),
            quote_mints: Some(vec!["stale".to_string()]),
        });
        assert_eq!(recovery.subscribe.underlying_mints, Some(Vec::new()));
        assert_eq!(recovery.subscribe.quote_mints, Some(Vec::new()));
    }

    #[test]
    fn recovery_removes_only_extra_channels_and_replaces_mint_scopes() {
        let desired = DesiredSubscriptions::from_initial(Some(&initial()));
        let recovery = desired.recovery(&SubscriptionsMessage {
            request_id: Uuid::new_v4(),
            channels: vec![WsChannel::Rfqs, WsChannel::Trades],
            underlying_mints: Some(vec!["old-underlying".to_string()]),
            quote_mints: Some(vec!["old-quote".to_string()]),
        });

        let unsubscribe = recovery.unsubscribe.expect("extra channel must be removed");
        assert_eq!(unsubscribe.channels, vec![WsChannel::Trades]);
        assert_eq!(recovery.subscribe.channels, vec![WsChannel::Rfqs]);
        assert_eq!(
            recovery.subscribe.underlying_mints,
            Some(vec!["underlying".to_string()])
        );
        assert_eq!(
            recovery.subscribe.quote_mints,
            Some(vec!["quote".to_string()])
        );
    }
}
