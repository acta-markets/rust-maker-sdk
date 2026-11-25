use std::future::Future;
use std::time::Duration;

use solana_pubkey::Pubkey;
use uuid::Uuid;

use crate::chain::vault::{
    ComputeBudgetBounds, Counterparty, VaultManager, VaultOpenTerms, VaultSpreadTerms,
    VaultTradeTerms,
};
use crate::chain::{ChainClient, ChainError, MarketInfo};
use crate::spreads::{SpreadError, SpreadType};
use crate::types::ids::OrderId;
use crate::types::{
    OptionTerms, Quantity, RfqAvailableAgainReason, RfqCloseReason, Strike, TakerSide,
    TimeoutSeconds,
};
use crate::ws::managed::ManagedReceiveError;
use crate::ws::types::{
    AcceptQuoteMessage, ClientMessage, Delivery, OrderState, QuoteReceivedMessage,
    RfqRequestMessage, RfqState, ServerMessage, SponsoredTxToSignData,
};

use super::client::{VaultOperatorClient, VaultRequestError};
use super::gate::SendGate;
use super::tx::VaultTxError;

/// An option or cash spread the vault trades as taker through an RFQ: on `Sell` it writes
/// (`OPEN_AS_WRITER`), on `Buy` it holds and pays the premium from its quote leg
/// (`BUY_FROM_MAKERS`).
#[derive(Debug, Clone)]
pub struct VaultTrade {
    pub side: TakerSide,
    pub market_pda: Pubkey,
    pub market: MarketInfo,
    /// Put products (cash-secured put, put spread) need a put market.
    pub terms: OptionTerms,
    pub quantity: Quantity,
    pub rfq_timeout: TimeoutSeconds,
    /// Bounds quote collection with `take`, and then serving plus `vault`, verification
    /// and signing; RFQ, accept and submit commands are bounded by the request timeout.
    pub window: Duration,
    pub bounds: ComputeBudgetBounds,
}

impl VaultTrade {
    /// The terms the vault signs to fill `quote`.
    pub(super) fn signed_terms(
        &self,
        quote: &QuoteReceivedMessage,
    ) -> Result<VaultOpenTerms, ChainError> {
        let market = &self.market;
        let spread = |kind, lower_strike: Strike, upper_strike: Strike| {
            VaultOpenTerms::Spread(VaultSpreadTerms {
                underlying_mint: market.underlying_mint,
                quote_mint: market.quote_mint,
                expiry_ts: market.expiry_ts,
                kind,
                side: quote.side,
                lower_strike: lower_strike.value(),
                upper_strike: upper_strike.value(),
                quantity: self.quantity.value(),
                price: quote.price.value(),
                valid_until: quote.valid_until.as_unix_seconds(),
                nonce: quote.nonce.value(),
            })
        };
        Ok(match self.terms {
            OptionTerms::CashPut { strike } => {
                spread(SpreadType::PutSpread, Strike::new(0), strike)
            }
            OptionTerms::CoveredCall { strike } | OptionTerms::CashSecuredPut { strike } => {
                VaultOpenTerms::Physical(VaultTradeTerms {
                    underlying_mint: market.underlying_mint,
                    quote_mint: market.quote_mint,
                    expiry_ts: market.expiry_ts,
                    is_put: market.is_put,
                    side: quote.side,
                    strike: strike.value(),
                    quantity: self.quantity.value(),
                    price: quote.price.value(),
                    valid_until: quote.valid_until.as_unix_seconds(),
                    nonce: quote.nonce.value(),
                })
            }
            OptionTerms::CallSpread {
                lower_strike,
                upper_strike,
            } => spread(SpreadType::CallSpread, lower_strike, upper_strike),
            OptionTerms::PutSpread {
                lower_strike,
                upper_strike,
            } => spread(SpreadType::PutSpread, lower_strike, upper_strike),
        })
    }
}

/// The bot's decisions and bookkeeping inside [`VaultOperatorClient::trade_rfq`].
pub trait TradeDesk: Send {
    type Error: std::fmt::Debug + std::fmt::Display + Send + Sync + 'static;

    /// Whether to take `quote`; asked for each quote until one is taken.
    fn take(
        &mut self,
        quote: &QuoteReceivedMessage,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    /// Runs before `AcceptQuote` is sent for the taken quote.
    fn accepting(&mut self, _quote: &QuoteReceivedMessage) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Re-checks the vault once the transaction is served and returns it to sign as.
    fn vault(&mut self) -> impl Future<Output = Result<VaultManager, Self::Error>> + Send;

    /// Runs with the signed transaction before it is submitted; an error leaves it unsubmitted.
    fn signed(&mut self, _trade: &SignedTrade) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A trade accepted by the server after the vault participant signed it.
///
/// The keeper still supplies transaction signature slot 0 and submits the transaction, so this
/// does not carry an on-chain transaction id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTrade {
    pub order_id: OrderId,
    /// Base58 signature made by the vault delegate or user participant over the served message.
    pub participant_signature: String,
    pub last_valid_block_height: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradeOutcome {
    /// No quote was taken within the window.
    Unfilled,
    /// The server accepted the participant-signed transaction for keeper cosigning; its chain
    /// outcome is pending.
    Submitted(SignedTrade),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeStage {
    Request,
    Take,
    Accept,
    Sign,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TradeError<E> {
    #[error("market_pda is not the program's market for these terms")]
    Market,
    #[error("spread terms: {0}")]
    Spread(SpreadError),
    #[error("trade desk: {0}")]
    Desk(E),
    #[error("{stage:?} failed")]
    Rejected {
        stage: TradeStage,
        #[source]
        error: VaultRequestError,
    },
    #[error(transparent)]
    Stream(#[from] ManagedReceiveError),
    #[error("the session changed during the trade")]
    SessionChanged,
    #[error("the accepted quote was not served within the window")]
    Unserved,
    /// The server failed the accepted order before serving it.
    #[error("the accepted order failed: {0}")]
    OrderFailed(String),
    /// The selection fell back and the RFQ is open again.
    #[error("the RFQ became available again: {0}")]
    Released(RfqAvailableAgainReason),
    #[error("the RFQ closed: {}", <&str>::from(.0))]
    Closed(RfqCloseReason),
    /// `take`, or `vault` plus verification and signing, outlived the window.
    #[error("{0:?} did not finish within the window")]
    TimedOut(TradeStage),
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Sign(#[from] VaultTxError),
    #[error("submitting the signed trade failed")]
    Submit {
        trade: SignedTrade,
        #[source]
        error: VaultRequestError,
    },
}

impl<E> TradeError<E> {
    /// The participant-signed trade this failure may have left with the keeper. `None`: nothing
    /// signed reached the server, so the trade cannot execute.
    #[must_use]
    pub fn in_flight(&self) -> Option<&SignedTrade> {
        match self {
            Self::Submit { trade, error } if error.delivery() != Some(Delivery::NotForwarded) => {
                Some(trade)
            }
            _ => None,
        }
    }
}

impl VaultOperatorClient {
    /// Trades `trade` in `connection_epoch`: requests the RFQ on its side, offers each quote to
    /// `desk`, accepts the first it takes, verifies and signs the served
    /// transaction against the chain counterparty, and submits it. Every send
    /// goes through `gate`.
    pub async fn trade_rfq<D: TradeDesk>(
        &self,
        chain: &ChainClient,
        trade: &VaultTrade,
        connection_epoch: u64,
        gate: &(impl SendGate + Sync),
        desk: &mut D,
    ) -> Result<TradeOutcome, TradeError<D::Error>> {
        let market = &trade.market;
        let canonical = crate::quote::market_pda(
            &chain.program_id(),
            &market.underlying_mint,
            &market.quote_mint,
            market.expiry_ts,
            market.is_put,
        );
        if canonical != trade.market_pda || trade.terms.position_type().is_put() != market.is_put {
            return Err(TradeError::Market);
        }
        if let Some(upper) = trade.terms.upper_strike() {
            let product = trade.terms.position_type();
            if product != crate::PositionType::CashPut && trade.terms.strike().value() == 0 {
                return Err(TradeError::Spread(SpreadError::InvalidStrikes));
            }
            crate::spreads::validate_strikes(
                SpreadType::try_from(u8::from(trade.terms.to_chain().position_type()))
                    .map_err(TradeError::Spread)?,
                trade.terms.to_chain().strike().value(),
                upper.value(),
            )
            .map_err(TradeError::Spread)?;
        }
        let buy = trade.side.is_taker_buy();
        if self
            .permissions()
            .is_some_and(|p| !if buy { p.can_buy() } else { p.can_sell() })
        {
            return Err(TradeError::Rejected {
                stage: TradeStage::Request,
                error: VaultRequestError::SideNotPermitted,
            });
        }
        let mut messages = self.subscribe_messages();
        let request_id = Uuid::new_v4();
        self.gated_command(
            gate,
            connection_epoch,
            ClientMessage::RfqRequest(RfqRequestMessage {
                request_id,
                market_pda: crate::MarketId::new(crate::Address32::new(
                    trade.market_pda.to_bytes(),
                )),
                terms: trade.terms,
                side: trade.side,
                quantity: trade.quantity,
                timeout_seconds: trade.rfq_timeout,
            }),
        )
        .await
        .map_err(|error| TradeError::Rejected {
            stage: TradeStage::Request,
            error,
        })?;

        let mut rfq_id = None;
        let mut taken: Option<QuoteReceivedMessage> = None;
        let mut deadline = tokio::time::Instant::now() + trade.window;
        loop {
            let Ok(inbound) = tokio::time::timeout_at(deadline, messages.recv()).await else {
                return match taken {
                    None => Ok(TradeOutcome::Unfilled),
                    Some(_) => Err(TradeError::Unserved),
                };
            };
            let inbound = inbound?;
            if inbound.connection_epoch != connection_epoch {
                return Err(TradeError::SessionChanged);
            }
            match inbound.message() {
                ServerMessage::RfqCreated(created) if created.request_id == request_id => {
                    rfq_id = Some(created.rfq_id);
                }
                ServerMessage::QuoteReceived(quote) if taken.is_none() => {
                    if Some(quote.rfq_id) == rfq_id
                        && quote.terms == trade.terms
                        && quote.side == trade.side
                        && tokio::time::timeout_at(deadline, desk.take(quote))
                            .await
                            .map_err(|_| TradeError::TimedOut(TradeStage::Take))?
                            .map_err(TradeError::Desk)?
                    {
                        desk.accepting(quote).map_err(TradeError::Desk)?;
                        self.gated_command(
                            gate,
                            connection_epoch,
                            ClientMessage::AcceptQuote(AcceptQuoteMessage {
                                request_id: Uuid::new_v4(),
                                rfq_id: quote.rfq_id,
                                maker_owner: quote.maker_owner,
                                order_id: quote.order_id,
                            }),
                        )
                        .await
                        .map_err(|error| TradeError::Rejected {
                            stage: TradeStage::Accept,
                            error,
                        })?;
                        taken = Some(quote.clone());
                        deadline = tokio::time::Instant::now() + trade.window;
                    }
                }
                ServerMessage::OrderUpdate(update) => {
                    if let (Some(quote), OrderState::Failed { reason }) = (&taken, &update.state)
                        && quote.order_id == update.order_id
                    {
                        return Err(TradeError::OrderFailed(reason.clone()));
                    }
                }
                ServerMessage::RfqUpdate(update) if Some(update.rfq_id) == rfq_id => {
                    match (&taken, &update.state) {
                        (Some(_), RfqState::Available { reason, .. }) => {
                            return Err(TradeError::Released(*reason));
                        }
                        (Some(_), RfqState::Closed { reason, .. }) => {
                            return Err(TradeError::Closed(*reason));
                        }
                        (None, RfqState::Closed { .. }) => return Ok(TradeOutcome::Unfilled),
                        _ => {}
                    }
                }
                ServerMessage::SponsoredTxToSign(served) => {
                    if let Some(quote) = taken.as_ref().filter(|q| q.order_id == served.order_id) {
                        let signed = tokio::time::timeout_at(
                            deadline,
                            self.sign_trade(chain, trade, quote, served, desk),
                        )
                        .await
                        .map_err(|_| TradeError::TimedOut(TradeStage::Sign))??;
                        return self
                            .submit_trade(gate, connection_epoch, signed, desk)
                            .await;
                    }
                }
                _ => {}
            }
        }
    }

    async fn sign_trade<D: TradeDesk>(
        &self,
        chain: &ChainClient,
        trade: &VaultTrade,
        quote: &QuoteReceivedMessage,
        served: &SponsoredTxToSignData,
        desk: &mut D,
    ) -> Result<(SignedTrade, crate::ws::types::SubmitSignedSponsoredTxData), TradeError<D::Error>>
    {
        let vault = desk.vault().await.map_err(TradeError::Desk)?;
        let maker = Pubkey::from(quote.maker_owner);
        let counterparty = Counterparty::load(chain, maker).await?;
        let keeper = chain.hot_authority().await?;
        let tables = chain
            .address_lookup_tables(
                &crate::chain::vault::served_lookup_table_keys(&served.tx_base64)
                    .map_err(VaultTxError::from)?,
            )
            .await?;
        let (submit, participant_signature) = self
            .verify_and_sign_sponsored_tx_with_participant_signature(
                served,
                &counterparty.open(&vault, keeper, trade.side, trade.signed_terms(quote)?),
                &tables,
                trade.bounds,
            )?;
        let signed = SignedTrade {
            order_id: served.order_id,
            participant_signature,
            last_valid_block_height: served.last_valid_block_height,
        };
        Ok((signed, submit))
    }

    async fn submit_trade<D: TradeDesk>(
        &self,
        gate: &(impl SendGate + Sync),
        connection_epoch: u64,
        (signed, submit): (SignedTrade, crate::ws::types::SubmitSignedSponsoredTxData),
        desk: &mut D,
    ) -> Result<TradeOutcome, TradeError<D::Error>> {
        desk.signed(&signed).map_err(TradeError::Desk)?;
        match self
            .gated_command(
                gate,
                connection_epoch,
                ClientMessage::SubmitSignedSponsoredTx(submit),
            )
            .await
        {
            Ok(()) => Ok(TradeOutcome::Submitted(signed)),
            Err(error) => Err(TradeError::Submit {
                trade: signed,
                error,
            }),
        }
    }
}
