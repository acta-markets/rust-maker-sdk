use std::future::Future;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use solana_sdk::pubkey::Pubkey;
use uuid::Uuid;

use crate::chain::vault::{ComputeBudgetBounds, Counterparty, VaultManager, VaultTradeTerms};
use crate::chain::{ChainClient, ChainError, MarketInfo};
use crate::types::ids::OrderId;
use crate::types::{PositionType, Quantity, Strike, TimeoutSeconds};
use crate::ws::managed::ManagedReceiveError;
use crate::ws::types::{
    AcceptQuoteMessage, ClientMessage, Delivery, QuoteReceivedMessage, RfqRequestMessage,
    ServerMessage, SponsoredTxToSignData,
};

use super::client::{VaultOperatorClient, VaultRequestError};
use super::gate::SendGate;
use super::tx::VaultTxError;

/// An option the vault writes and sells through an RFQ.
#[derive(Debug, Clone)]
pub struct VaultSale {
    pub market_pda: Pubkey,
    pub market: MarketInfo,
    pub strike: Strike,
    pub quantity: Quantity,
    pub rfq_timeout: TimeoutSeconds,
    /// Bounds quote collection with `take`, and then serving plus `writer`, verification
    /// and signing; RFQ, accept and submit commands are bounded by the request timeout.
    pub window: Duration,
    pub bounds: ComputeBudgetBounds,
}

/// The bot's decisions and bookkeeping inside [`VaultOperatorClient::sell_rfq`].
pub trait SaleDesk: Send {
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

    /// Re-checks the vault once the transaction is served and returns the writer to sign as.
    fn writer(&mut self) -> impl Future<Output = Result<VaultManager, Self::Error>> + Send;

    /// Runs with the signed transaction before it is submitted; an error leaves it unsubmitted.
    fn signed(&mut self, _sale: &SignedSale) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A signed sale transaction; `signature` is its transaction id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedSale {
    pub order_id: OrderId,
    pub signature: String,
    pub last_valid_block_height: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaleOutcome {
    /// No quote was taken within the window.
    Unfilled,
    /// The server accepted the signed transaction; its chain outcome is pending.
    Submitted(SignedSale),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaleStage {
    Request,
    Take,
    Accept,
    Sign,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SaleError<E> {
    #[error("market_pda is not the program's market for these terms")]
    Market,
    #[error("sale desk: {0}")]
    Desk(E),
    #[error("{stage:?} failed")]
    Rejected {
        stage: SaleStage,
        #[source]
        error: VaultRequestError,
    },
    #[error(transparent)]
    Stream(#[from] ManagedReceiveError),
    #[error("the session changed during the sale")]
    SessionChanged,
    #[error("the accepted quote was not served within the window")]
    Unserved,
    /// `take`, or `writer` plus verification and signing, outlived the window.
    #[error("{0:?} did not finish within the window")]
    TimedOut(SaleStage),
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Sign(#[from] VaultTxError),
    #[error("submitting the signed sale failed")]
    Submit {
        sale: SignedSale,
        #[source]
        error: VaultRequestError,
    },
}

impl<E> SaleError<E> {
    /// The signed sale this failure may have left in flight. `None`: nothing
    /// signed reached the server, so the sale cannot execute.
    #[must_use]
    pub fn in_flight(&self) -> Option<&SignedSale> {
        match self {
            Self::Submit { sale, error } if error.delivery() != Some(Delivery::NotForwarded) => {
                Some(sale)
            }
            _ => None,
        }
    }
}

impl VaultOperatorClient {
    /// Sells `sale` in `connection_epoch`: requests the RFQ, offers each quote to
    /// `desk`, accepts the first it takes, verifies and signs the served
    /// transaction against the chain counterparty, and submits it. Every send
    /// goes through `gate`.
    pub async fn sell_rfq<D: SaleDesk>(
        &self,
        chain: &ChainClient,
        sale: &VaultSale,
        connection_epoch: u64,
        gate: &(impl SendGate + Sync),
        desk: &mut D,
    ) -> Result<SaleOutcome, SaleError<D::Error>> {
        let market = &sale.market;
        let canonical = crate::quote::market_pda(
            &chain.program_id(),
            &market.underlying_mint,
            &market.quote_mint,
            market.expiry_ts,
            market.is_put,
        );
        if canonical != sale.market_pda {
            return Err(SaleError::Market);
        }
        let mut messages = self.subscribe_messages();
        let request_id = Uuid::new_v4();
        self.gated_command(
            gate,
            connection_epoch,
            ClientMessage::RfqRequest(RfqRequestMessage {
                request_id,
                market: sale.market_pda.to_string().into(),
                position_type: if market.is_put {
                    PositionType::CashSecuredPut
                } else {
                    PositionType::CoveredCall
                },
                strike: sale.strike,
                quantity: sale.quantity,
                timeout_seconds: sale.rfq_timeout,
            }),
        )
        .await
        .map_err(|error| SaleError::Rejected {
            stage: SaleStage::Request,
            error,
        })?;

        let mut rfq_id = None;
        let mut taken: Option<QuoteReceivedMessage> = None;
        let mut deadline = tokio::time::Instant::now() + sale.window;
        loop {
            let Ok(inbound) = tokio::time::timeout_at(deadline, messages.recv()).await else {
                return match taken {
                    None => Ok(SaleOutcome::Unfilled),
                    Some(_) => Err(SaleError::Unserved),
                };
            };
            let inbound = inbound?;
            if inbound.connection_epoch != connection_epoch {
                return Err(SaleError::SessionChanged);
            }
            match inbound.message() {
                ServerMessage::RfqCreated(created) if created.request_id == Some(request_id) => {
                    rfq_id = Some(created.rfq_id);
                }
                ServerMessage::QuoteReceived(quote) if taken.is_none() => {
                    if Some(quote.rfq_id) == rfq_id
                        && quote.strike == sale.strike
                        && tokio::time::timeout_at(deadline, desk.take(quote))
                            .await
                            .map_err(|_| SaleError::TimedOut(SaleStage::Take))?
                            .map_err(SaleError::Desk)?
                    {
                        desk.accepting(quote).map_err(SaleError::Desk)?;
                        self.gated_command(
                            gate,
                            connection_epoch,
                            ClientMessage::AcceptQuote(AcceptQuoteMessage {
                                request_id: Uuid::new_v4(),
                                rfq_id: quote.rfq_id,
                                maker: quote.maker.clone(),
                                order_id: quote.order_id,
                            }),
                        )
                        .await
                        .map_err(|error| SaleError::Rejected {
                            stage: SaleStage::Accept,
                            error,
                        })?;
                        taken = Some(quote.clone());
                        deadline = tokio::time::Instant::now() + sale.window;
                    }
                }
                ServerMessage::SponsoredTxToSign(served) => {
                    if let Some(quote) = taken.as_ref().filter(|q| q.order_id == served.order_id) {
                        let signed = tokio::time::timeout_at(
                            deadline,
                            self.sign_sale(chain, sale, quote, served, desk),
                        )
                        .await
                        .map_err(|_| SaleError::TimedOut(SaleStage::Sign))??;
                        return self.submit_sale(gate, connection_epoch, signed, desk).await;
                    }
                }
                _ => {}
            }
        }
    }

    async fn sign_sale<D: SaleDesk>(
        &self,
        chain: &ChainClient,
        sale: &VaultSale,
        quote: &QuoteReceivedMessage,
        served: &SponsoredTxToSignData,
        desk: &mut D,
    ) -> Result<(SignedSale, crate::ws::types::SubmitSignedSponsoredTxData), SaleError<D::Error>>
    {
        let writer = desk.writer().await.map_err(SaleError::Desk)?;
        let maker = quote
            .maker
            .parse::<Pubkey>()
            .map_err(|_| ChainError::Identity("quote maker"))?;
        let counterparty = Counterparty::load(chain, maker).await?;
        let terms = VaultTradeTerms {
            underlying_mint: sale.market.underlying_mint,
            quote_mint: sale.market.quote_mint,
            expiry_ts: sale.market.expiry_ts,
            is_put: sale.market.is_put,
            strike: sale.strike.value(),
            quantity: sale.quantity.value(),
            price: quote.price.value(),
            valid_until: quote.valid_until.as_unix_seconds(),
            nonce: quote.nonce.value(),
        };
        let tables = chain
            .address_lookup_tables(
                &crate::chain::vault::served_lookup_table_keys(&served.tx_base64)
                    .map_err(VaultTxError::from)?,
            )
            .await?;
        let submit = self.verify_and_sign_sponsored_tx(
            served,
            &counterparty.sale(&writer, terms),
            &tables,
            sale.bounds,
        )?;
        let signed = SignedSale {
            order_id: served.order_id,
            signature: transaction_id(&submit.tx_base64)?,
            last_valid_block_height: served.last_valid_block_height,
        };
        Ok((signed, submit))
    }

    async fn submit_sale<D: SaleDesk>(
        &self,
        gate: &(impl SendGate + Sync),
        connection_epoch: u64,
        (signed, submit): (SignedSale, crate::ws::types::SubmitSignedSponsoredTxData),
        desk: &mut D,
    ) -> Result<SaleOutcome, SaleError<D::Error>> {
        desk.signed(&signed).map_err(SaleError::Desk)?;
        match self
            .gated_command(
                gate,
                connection_epoch,
                ClientMessage::SubmitSignedSponsoredTx(submit),
            )
            .await
        {
            Ok(()) => Ok(SaleOutcome::Submitted(signed)),
            Err(error) => Err(SaleError::Submit {
                sale: signed,
                error,
            }),
        }
    }
}

/// First signature of a signed wire transaction, base58.
fn transaction_id(tx_base64: &str) -> Result<String, VaultTxError> {
    let malformed = || {
        VaultTxError::Intent(crate::chain::vault::VaultIntentError::Malformed(
            "signed transaction",
        ))
    };
    let bytes = STANDARD.decode(tx_base64).map_err(|_| malformed())?;
    match bytes.first() {
        Some(count) if (1..0x80).contains(count) => bytes
            .get(1..65)
            .map(crate::wire::encode_base58)
            .ok_or_else(malformed),
        _ => Err(malformed()),
    }
}
