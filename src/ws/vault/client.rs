use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::orders::SignerLike;
use crate::quote::{QuoteBuildError, QuoteBuilder};
use crate::types::VaultDelegatePermissions;
use crate::types::ids::OrderId;
use crate::wire::{WireError, decode_base58_32};
use crate::ws::managed::{
    ManagedMessageReceiver, ManagedWsConfigError, ManagedWsError, ManagedWsEvent, ManagedWsHandle,
    ManagedWsState, ReferenceChangeReceiver, SendAwaitError, SendTicket, WaitUntilReadyError,
    spawn_managed_ws,
};
use crate::ws::types::{
    AcceptQuoteMessage, BatchQuotesMessage, CancelAllQuotesMessage, CancelQuoteData, CancelRfqData,
    ClientMessage, Delivery, GetSwapStatusMessage, QuoteMessage, ReplaceQuoteMessage,
    RfqRequestMessage, ServerError, ServerMessage, SponsoredTxToSignData,
    SubmitSignedSponsoredTxData, SubscribeData, SwapQuoteData, SwapQuoteResultData,
    SwapStatusMessage, SwapSubmitData, SwapSubmitResultData, UnsubscribeData, VaultAuthSuccessData,
};

use solana_sdk::message::AddressLookupTableAccount;

use crate::chain::vault::{
    ComputeBudgetBounds, VaultOpenIntent, VaultSwapIntent, verify_vault_open, verify_vault_swap,
};

use super::config::VaultOperatorConfig;
use super::gate::SendGate;
use super::projection::VaultProjectionReceiver;
use super::tx::{VaultTxError, ensure_deadline_ahead, ensure_system_time_ahead};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultClientError {
    #[error("vault_pda is not a base58 32-byte address")]
    InvalidVaultPda(#[source] WireError),
    #[error(transparent)]
    InvalidConfig(#[from] ManagedWsConfigError),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultRequestError {
    #[error(transparent)]
    Session(#[from] ManagedWsError),
    #[error(transparent)]
    Transport(#[from] SendAwaitError),
    #[error("server rejected request {request_id}")]
    Rejected {
        request_id: Uuid,
        response: Arc<ServerMessage>,
    },
    #[error("expected {expected}, received a different correlated response")]
    UnexpectedResponse {
        expected: &'static str,
        response: Arc<ServerMessage>,
    },
    #[error("the send gate refused the command")]
    Gated,
}

impl VaultRequestError {
    #[must_use]
    pub fn server_error(&self) -> Option<&ServerError> {
        match self {
            Self::Rejected { response, .. } => match &**response {
                ServerMessage::RequestError(error) => Some(&error.error),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether the command can have reached the server; `None` is a rejection
    /// of a command without delivery semantics, which did not execute.
    #[must_use]
    pub fn delivery(&self) -> Option<Delivery> {
        match self {
            Self::Rejected { response, .. } => match &**response {
                ServerMessage::RequestError(error) => error.delivery,
                _ => Some(Delivery::Unknown),
            },
            Self::Session(ManagedWsError::NotReady | ManagedWsError::QueueFull)
            | Self::Gated
            | Self::Transport(
                SendAwaitError::SubscriptionPending
                | SendAwaitError::NotReady
                | SendAwaitError::NoCorrelationKey
                | SendAwaitError::DuplicateInFlight
                | SendAwaitError::TooManyPending { .. }
                | SendAwaitError::QueueFull
                | SendAwaitError::InvalidMessage(_),
            ) => Some(Delivery::NotForwarded),
            Self::Session(_) | Self::Transport(_) | Self::UnexpectedResponse { .. } => {
                Some(Delivery::Unknown)
            }
        }
    }
}

/// Vault operator session on `/vault`; every reconnect re-runs `StartVaultAuth`.
#[derive(Clone)]
pub struct VaultOperatorClient {
    inner: ManagedWsHandle,
    vault_pda: String,
    vault_owner: [u8; 32],
    operator: Arc<dyn SignerLike + Send + Sync>,
    request_timeout: Duration,
}

impl VaultOperatorClient {
    pub fn spawn(config: VaultOperatorConfig) -> Result<Self, VaultClientError> {
        let vault_owner =
            decode_base58_32(&config.vault_pda).map_err(VaultClientError::InvalidVaultPda)?;
        Ok(Self {
            inner: spawn_managed_ws(config.managed)?,
            vault_pda: config.vault_pda,
            vault_owner,
            operator: config.operator,
            request_timeout: config.request_timeout,
        })
    }

    #[must_use]
    pub fn vault_pda(&self) -> &str {
        &self.vault_pda
    }

    /// Vault address used as `maker` in the order preimage.
    #[must_use]
    pub const fn vault_owner(&self) -> [u8; 32] {
        self.vault_owner
    }

    #[must_use]
    pub fn operator_pubkey(&self) -> String {
        self.operator.pubkey_base58()
    }

    /// `None` before authentication and after it ends.
    #[must_use]
    pub fn auth(&self) -> Option<VaultAuthSuccessData> {
        self.inner.vault_auth()
    }

    #[must_use]
    pub fn permissions(&self) -> Option<VaultDelegatePermissions> {
        self.auth().map(|auth| auth.permissions)
    }

    #[must_use]
    pub fn can_maker(&self) -> bool {
        self.inner.is_ready()
            && self.auth().is_some_and(|auth| {
                matches!(
                    auth.operator_kind,
                    crate::types::VaultOperatorKind::Delegate
                        | crate::types::VaultOperatorKind::Governance
                ) && auth.permissions.can_maker()
            })
    }

    #[must_use]
    pub fn can_taker(&self) -> bool {
        self.inner.is_ready()
            && self
                .permissions()
                .is_some_and(VaultDelegatePermissions::can_taker)
    }

    #[must_use]
    pub fn subscribe_vault_auth(&self) -> watch::Receiver<Option<VaultAuthSuccessData>> {
        self.inner.subscribe_vault_auth()
    }

    pub fn sign_quote(&self, quote: &QuoteBuilder<'_>) -> Result<QuoteMessage, QuoteBuildError> {
        quote
            .clone()
            .maker_owner(self.vault_owner)
            .sign(self.operator.as_ref())
    }

    /// Send only in `connection_epoch`, the epoch whose recovery the strategy
    /// applied; a reconnect in between rejects the quote with `NotReady`.
    pub async fn quote_in_epoch(
        &self,
        payload: QuoteMessage,
        connection_epoch: u64,
    ) -> Result<(), ManagedWsError> {
        self.inner
            .send_in_epoch(ClientMessage::Quote(payload), connection_epoch)
            .await
    }

    pub fn try_quote_in_epoch(
        &self,
        payload: QuoteMessage,
        connection_epoch: u64,
    ) -> Result<SendTicket, ManagedWsError> {
        self.inner
            .try_send_in_epoch(ClientMessage::Quote(payload), connection_epoch)
    }

    pub async fn replace_in_epoch(
        &self,
        payload: ReplaceQuoteMessage,
        connection_epoch: u64,
    ) -> Result<(), ManagedWsError> {
        self.inner
            .send_in_epoch(ClientMessage::ReplaceQuote(payload), connection_epoch)
            .await
    }

    pub async fn batch_quotes_in_epoch(
        &self,
        payload: BatchQuotesMessage,
        connection_epoch: u64,
    ) -> Result<(), ManagedWsError> {
        self.inner
            .send_in_epoch(ClientMessage::BatchQuotes(payload), connection_epoch)
            .await
    }

    pub async fn cancel_quote(&self, payload: CancelQuoteData) -> Result<(), ManagedWsError> {
        self.send(ClientMessage::CancelQuote(payload)).await
    }

    pub async fn cancel_all_quotes(
        &self,
        payload: CancelAllQuotesMessage,
    ) -> Result<(), ManagedWsError> {
        self.send(ClientMessage::CancelAllQuotes(payload)).await
    }

    /// Resolves on the server's receipt; `RfqCreated` still arrives as a push.
    pub async fn rfq_request(&self, payload: RfqRequestMessage) -> Result<(), VaultRequestError> {
        self.command(ClientMessage::RfqRequest(payload)).await
    }

    pub async fn cancel_rfq(&self, payload: CancelRfqData) -> Result<(), ManagedWsError> {
        self.send(ClientMessage::CancelRfq(payload)).await
    }

    pub async fn accept_quote(&self, payload: AcceptQuoteMessage) -> Result<(), VaultRequestError> {
        self.command(ClientMessage::AcceptQuote(payload)).await
    }

    /// Signs only if the served transaction opens exactly `intent` within `bounds`.
    pub fn verify_and_sign_sponsored_tx(
        &self,
        to_sign: &SponsoredTxToSignData,
        intent: &VaultOpenIntent<'_>,
        tables: &[AddressLookupTableAccount],
        bounds: ComputeBudgetBounds,
    ) -> Result<SubmitSignedSponsoredTxData, VaultTxError> {
        let transaction = verify_vault_open(to_sign, intent, tables, bounds)?;
        ensure_system_time_ahead("signature_deadline", to_sign.signature_deadline)?;
        Ok(SubmitSignedSponsoredTxData {
            request_id: Uuid::new_v4(),
            order_id: to_sign.order_id,
            tx_base64: transaction.sign(self.operator.as_ref())?,
        })
    }

    pub async fn submit_signed_sponsored_tx(
        &self,
        payload: SubmitSignedSponsoredTxData,
    ) -> Result<(), VaultRequestError> {
        self.command(ClientMessage::SubmitSignedSponsoredTx(payload))
            .await
    }

    pub async fn swap_quote(
        &self,
        payload: SwapQuoteData,
    ) -> Result<SwapQuoteResultData, VaultRequestError> {
        self.request(
            ClientMessage::SwapQuote(payload),
            "swap_quote_result",
            |message| match message {
                ServerMessage::SwapQuoteResult(data) => Some(data.clone()),
                _ => None,
            },
        )
        .await
    }

    /// Signs only if the served swap carries exactly the requested and quoted terms within `bounds`.
    pub fn verify_and_sign_swap(
        &self,
        intent: &VaultSwapIntent<'_>,
        tables: &[AddressLookupTableAccount],
        bounds: ComputeBudgetBounds,
    ) -> Result<SwapSubmitData, VaultTxError> {
        let transaction = verify_vault_swap(intent, tables, bounds)?;
        ensure_deadline_ahead("valid_until", &intent.quote.valid_until)?;
        Ok(SwapSubmitData {
            request_id: Uuid::new_v4(),
            execution_id: intent.quote.execution_id,
            signed_tx_base64: transaction.sign(self.operator.as_ref())?,
        })
    }

    /// Submits only in `connection_epoch` and only if `gate` admits it; resolves on
    /// the durable enqueue receipt. [`VaultRequestError::delivery`] classifies failures.
    pub async fn swap_submit_in_epoch(
        &self,
        payload: SwapSubmitData,
        connection_epoch: u64,
        gate: &impl SendGate,
    ) -> Result<SwapSubmitResultData, VaultRequestError> {
        self.gated_request(
            gate,
            connection_epoch,
            ClientMessage::SwapSubmit(payload),
            "swap_submit_result",
            |message| match message {
                ServerMessage::SwapSubmitResult(data) => Some(data.clone()),
                _ => None,
            },
        )
        .await
    }

    /// Terminal outcomes also arrive as `SwapSettled` pushes.
    pub async fn swap_status(
        &self,
        execution_id: OrderId,
    ) -> Result<SwapStatusMessage, VaultRequestError> {
        self.request(
            ClientMessage::GetSwapStatus(GetSwapStatusMessage {
                request_id: Uuid::new_v4(),
                execution_id,
            }),
            "swap_status",
            |message| match message {
                ServerMessage::SwapStatus(data) => Some(data.clone()),
                _ => None,
            },
        )
        .await
    }

    pub async fn subscribe(&self, payload: SubscribeData) -> Result<(), ManagedWsError> {
        self.send(ClientMessage::Subscribe(payload)).await
    }

    pub async fn unsubscribe(&self, payload: UnsubscribeData) -> Result<(), ManagedWsError> {
        self.send(ClientMessage::Unsubscribe(payload)).await
    }

    pub fn subscribe_messages(&self) -> ManagedMessageReceiver {
        self.inner.subscribe_messages()
    }

    pub fn subscribe_reference_changes(&self) -> ReferenceChangeReceiver {
        self.inner.subscribe_reference_changes()
    }

    pub fn subscribe_projections(&self) -> VaultProjectionReceiver {
        VaultProjectionReceiver::new(self.inner.subscribe_messages())
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ManagedWsEvent> {
        self.inner.subscribe_events()
    }

    #[must_use]
    pub fn state(&self) -> ManagedWsState {
        self.inner.state()
    }

    pub async fn wait_until_ready(&self) -> Result<u64, WaitUntilReadyError> {
        self.inner.wait_until_ready().await
    }

    #[must_use]
    pub const fn raw(&self) -> &ManagedWsHandle {
        &self.inner
    }

    #[must_use]
    pub fn into_raw(self) -> ManagedWsHandle {
        self.inner
    }

    pub async fn close(&self) -> Result<(), ManagedWsError> {
        self.inner.close().await
    }

    async fn send(&self, message: ClientMessage) -> Result<(), ManagedWsError> {
        self.inner.ensure_ready()?;
        self.inner.send(message).await
    }

    async fn command(&self, message: ClientMessage) -> Result<(), VaultRequestError> {
        self.request(message, "command_accepted", |message| match message {
            ServerMessage::CommandAccepted(_) => Some(()),
            _ => None,
        })
        .await
    }

    #[cfg(feature = "chain-rpc")]
    pub(super) async fn gated_command(
        &self,
        gate: &impl SendGate,
        connection_epoch: u64,
        message: ClientMessage,
    ) -> Result<(), VaultRequestError> {
        self.gated_request(
            gate,
            connection_epoch,
            message,
            "command_accepted",
            |message| match message {
                ServerMessage::CommandAccepted(_) => Some(()),
                _ => None,
            },
        )
        .await
    }

    async fn gated_request<T>(
        &self,
        gate: &impl SendGate,
        connection_epoch: u64,
        message: ClientMessage,
        expected: &'static str,
        extract: fn(&ServerMessage) -> Option<T>,
    ) -> Result<T, VaultRequestError> {
        let response = gate
            .admit(connection_epoch, || {
                self.inner
                    .try_send_await_in_epoch(message, self.request_timeout, connection_epoch)
            })
            .ok_or(VaultRequestError::Gated)??
            .await?;
        Self::correlated(response, expected, extract)
    }

    async fn request<T>(
        &self,
        message: ClientMessage,
        expected: &'static str,
        extract: fn(&ServerMessage) -> Option<T>,
    ) -> Result<T, VaultRequestError> {
        self.inner.ensure_ready()?;
        let response = self.inner.send_await(message, self.request_timeout).await?;
        Self::correlated(response, expected, extract)
    }

    fn correlated<T>(
        response: Arc<ServerMessage>,
        expected: &'static str,
        extract: fn(&ServerMessage) -> Option<T>,
    ) -> Result<T, VaultRequestError> {
        if let ServerMessage::RequestError(error) = &*response {
            return Err(VaultRequestError::Rejected {
                request_id: error.request_id,
                response,
            });
        }
        extract(&response).ok_or(VaultRequestError::UnexpectedResponse { expected, response })
    }
}
