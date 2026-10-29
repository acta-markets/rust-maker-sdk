//! Quote assembly: [`RfqBinding`] decodes identities and resolves offered terms;
//! [`QuoteBuilder`] derives the signed preimage and [`QuoteMessage`] from the
//! same terms, price, expiry and nonce.

use crate::orders::{OrderPreimageArgs, SignerLike, compute_order_id, sign_order_id_with_signer};
use crate::signing::{AsyncSignerLike, SigningError};
use crate::types::ids::{Nonce, OrderId, Price, Quantity};
use crate::types::{OptionTerms, PositionType, QuoteExpiry, TakerSide};
use crate::wire::{WireError, encode_base58};
use crate::ws::types::{QuoteMessage, ReplaceQuoteMessage, RfqBroadcastMessage};
use std::time::Duration;
use uuid::Uuid;

/// A quote could not be assembled from the RFQ and the supplied values.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum QuoteBuildError {
    #[error(transparent)]
    Order(#[from] crate::orders::OrderError),
    #[error(transparent)]
    Spread(#[from] crate::spreads::SpreadError),
    #[error("rfq field `{field}` is not a valid base58 32-byte key: {source}")]
    Identity {
        field: &'static str,
        #[source]
        source: WireError,
    },

    #[error("rfq market does not match trusted signing terms: {0}")]
    MarketBinding(&'static str),

    /// Terms absent from `RfqBroadcast.terms` and `order_options`.
    #[error("terms {requested:?} are not offered by this rfq; offered: {offered:?}")]
    TermsNotOffered {
        requested: OptionTerms,
        offered: Vec<OptionTerms>,
    },

    #[error("quote is missing `{0}`")]
    Missing(&'static str),

    #[error("valid_until {valid_until} is after market expiry {market_expiry}")]
    ValidUntilAfterMarketExpiry {
        valid_until: QuoteExpiry,
        market_expiry: QuoteExpiry,
    },

    #[error("valid_until {valid_until} is before the venue minimum {min_valid_until}")]
    ValidUntilTooSoon {
        valid_until: QuoteExpiry,
        min_valid_until: QuoteExpiry,
    },

    /// `valid_until − settlement_buffer_seconds` is not after now, so the
    /// quote could never be accepted.
    #[error("valid_until {valid_until} is within the settlement buffer ending at {buffer_end}")]
    ValidUntilWithinSettlementBuffer {
        valid_until: QuoteExpiry,
        buffer_end: QuoteExpiry,
    },

    #[error("signing failed: {0}")]
    Signing(#[from] SigningError),

    #[error("taker_buy_unsupported")]
    TakerBuyUnsupported,
}

/// An RFQ with its identities decoded and its offered terms resolved.
///
/// Build once per RFQ and reuse it for every quote on that RFQ.
#[derive(Debug, Clone)]
pub struct RfqBinding {
    rfq_id: Uuid,
    chain_id: u64,
    program_id: [u8; 32],
    market: [u8; 32],
    market_expiry: QuoteExpiry,
    taker: [u8; 32],
    terms: OptionTerms,
    quantity: Quantity,
    offered_terms: Vec<OptionTerms>,
    side: TakerSide,
}

impl RfqBinding {
    /// Decode and validate an inbound [`RfqBroadcastMessage`].
    ///
    /// # Errors
    /// Rejects malformed identities and inconsistent market descriptors. The
    /// program is pinned to [`crate::ACTA_PROGRAM_ID`], not the WS endpoint.
    pub fn from_broadcast(rfq: &RfqBroadcastMessage) -> Result<Self, QuoteBuildError> {
        Self::from_broadcast_for_program(rfq, crate::ACTA_PROGRAM_ID)
    }

    /// Use a caller-configured deployment ID, never an ID copied from the RFQ.
    ///
    /// # Errors
    /// Rejects identities or market terms inconsistent with the expected program.
    pub fn from_broadcast_for_program(
        rfq: &RfqBroadcastMessage,
        expected_program: [u8; 32],
    ) -> Result<Self, QuoteBuildError> {
        let side = rfq.side;
        if side.is_taker_buy() {
            return Err(QuoteBuildError::TakerBuyUnsupported);
        }
        let terms = rfq.terms;
        if terms.is_spread() {
            let upper = terms
                .upper_strike()
                .ok_or(QuoteBuildError::MarketBinding("spread terms"))?;
            if terms.position_type() != PositionType::CashPut && terms.strike().value() == 0 {
                return Err(crate::spreads::SpreadError::InvalidStrikes.into());
            }
            crate::spreads::validate_strikes(
                crate::spreads::SpreadType::try_from(u8::from(terms.to_chain().position_type()))?,
                terms.to_chain().strike().value(),
                upper.value(),
            )?;
            if rfq.quantity.value() == 0
                || rfq.order_options.len() != 1
                || rfq.order_options[0] != terms
            {
                return Err(QuoteBuildError::MarketBinding("spread pair"));
            }
        } else if rfq
            .order_options
            .iter()
            .any(|option| option.is_spread() || option.position_type() != terms.position_type())
        {
            return Err(QuoteBuildError::MarketBinding("physical order_options"));
        }
        let (market, market_expiry) = validate_market_binding(rfq, expected_program)?;
        let mut offered_terms = Vec::with_capacity(1 + rfq.order_options.len());
        offered_terms.push(terms);
        offered_terms.extend(
            rfq.order_options
                .iter()
                .copied()
                .filter(|option| *option != terms),
        );

        Ok(Self {
            rfq_id: rfq.rfq_id,
            chain_id: rfq.market.chain_id.value(),
            program_id: expected_program,
            market,
            market_expiry,
            taker: rfq.taker.0,
            terms,
            quantity: rfq.quantity,
            offered_terms,
            side,
        })
    }

    #[must_use]
    pub const fn rfq_id(&self) -> Uuid {
        self.rfq_id
    }

    /// RFQ quantity used for pricing and included in the signed preimage.
    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    #[must_use]
    pub const fn position_type(&self) -> PositionType {
        self.terms.position_type()
    }

    /// Complete terms selected by the taker for this RFQ.
    #[must_use]
    pub const fn terms(&self) -> OptionTerms {
        self.terms
    }

    #[must_use]
    pub const fn side(&self) -> TakerSide {
        self.side
    }

    /// Latest `valid_until` the venue accepts for this RFQ.
    #[must_use]
    pub const fn market_expiry(&self) -> QuoteExpiry {
        self.market_expiry
    }

    /// Every term set this RFQ will accept a quote on.
    #[must_use]
    pub fn offered_terms(&self) -> &[OptionTerms] {
        &self.offered_terms
    }

    /// Start a quote on this RFQ's requested terms.
    pub const fn quote(&self) -> QuoteBuilder<'_> {
        QuoteBuilder {
            binding: self,
            terms: self.terms,
            price: None,
            valid_until: None,
            min_valid_until: None,
            buffer_end: None,
            nonce: None,
            maker: None,
        }
    }
}

fn validate_market_binding(
    rfq: &RfqBroadcastMessage,
    expected_program: [u8; 32],
) -> Result<([u8; 32], QuoteExpiry), QuoteBuildError> {
    use QuoteBuildError::MarketBinding;
    use solana_pubkey::Pubkey;

    let descriptor = &rfq.market;
    if descriptor.chain_id.value() != 0 {
        return Err(MarketBinding("chain_id"));
    }
    if descriptor.program_id.0 != expected_program {
        return Err(MarketBinding("program_id"));
    }
    if descriptor.is_put != rfq.terms.position_type().is_put() {
        return Err(MarketBinding("terms"));
    }
    let underlying = descriptor.underlying_mint.0;
    let quote = descriptor.quote_mint.0;
    let (collateral, settlement) = if rfq.terms.is_spread() {
        (quote, quote)
    } else if descriptor.is_put {
        (quote, underlying)
    } else {
        (underlying, quote)
    };
    if descriptor.collateral_mint.0 != collateral || descriptor.settlement_mint.0 != settlement {
        return Err(MarketBinding("collateral/settlement mint"));
    }
    let expiry = descriptor
        .expiry_ts
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| MarketBinding("expiry_ts"))?;
    if expiry.subsec_nanos() != 0 || expiry.as_secs() > i64::MAX as u64 {
        return Err(MarketBinding("expiry_ts"));
    }
    let expected_market = market_pda(
        &Pubkey::new_from_array(expected_program),
        &Pubkey::new_from_array(underlying),
        &Pubkey::new_from_array(quote),
        expiry.as_secs(),
        descriptor.is_put,
    )
    .to_bytes();
    let market = descriptor.market_pda.0;
    if market != expected_market {
        return Err(MarketBinding("market_pda"));
    }
    Ok((market, QuoteExpiry::from_unix_seconds(expiry.as_secs())))
}

pub(crate) fn market_pda(
    program_id: &solana_pubkey::Pubkey,
    underlying_mint: &solana_pubkey::Pubkey,
    quote_mint: &solana_pubkey::Pubkey,
    expiry_ts: u64,
    is_put: bool,
) -> solana_pubkey::Pubkey {
    solana_pubkey::Pubkey::find_program_address(
        &[
            b"market",
            underlying_mint.as_ref(),
            quote_mint.as_ref(),
            &expiry_ts.to_le_bytes(),
            &[u8::from(is_put)],
        ],
        program_id,
    )
    .0
}

#[derive(Debug, Clone, Copy)]
struct Resolved {
    price: Price,
    valid_until: QuoteExpiry,
    nonce: Nonce,
    maker: [u8; 32],
}

/// Accumulates the maker's side of a quote, then signs it.
#[derive(Debug, Clone)]
#[must_use = "a builder does nothing until it is signed"]
pub struct QuoteBuilder<'a> {
    binding: &'a RfqBinding,
    terms: OptionTerms,
    price: Option<Price>,
    valid_until: Option<QuoteExpiry>,
    min_valid_until: Option<QuoteExpiry>,
    buffer_end: Option<QuoteExpiry>,
    nonce: Option<Nonce>,
    maker: Option<[u8; 32]>,
}

impl QuoteBuilder<'_> {
    /// Terms this builder will bind into the quote and signed preimage.
    #[must_use]
    pub const fn selected_terms(&self) -> OptionTerms {
        self.terms
    }

    /// Quote alternative terms offered by the RFQ instead of its requested terms.
    pub const fn terms(mut self, terms: OptionTerms) -> Self {
        self.terms = terms;
        self
    }

    pub const fn price(mut self, price: Price) -> Self {
        self.price = Some(price);
        self
    }

    pub const fn valid_until(mut self, valid_until: QuoteExpiry) -> Self {
        self.valid_until = Some(valid_until);
        self
    }

    /// Earliest `valid_until` the venue accepts: now plus `min_quote_validity_seconds`
    /// from `GET /api/v1/venue`.
    pub const fn min_valid_until(mut self, min_valid_until: QuoteExpiry) -> Self {
        self.min_valid_until = Some(min_valid_until);
        self
    }

    /// Apply the venue's quote window at the current clock: the floor
    /// `now + min_quote_validity_seconds`, and `valid_until − settlement_buffer_seconds`
    /// must be after now.
    #[cfg(feature = "http")]
    pub fn venue(self, venue: &crate::http::Venue) -> Self {
        self.quote_window(
            venue.min_quote_validity_seconds,
            venue.settlement_buffer_seconds,
        )
    }

    /// [`Self::venue`] from the session's `Welcome` instead of HTTP.
    pub fn welcome(self, welcome: &crate::ws::types::WelcomeData) -> Self {
        self.quote_window(
            welcome.min_quote_validity_seconds,
            welcome.settlement_buffer_seconds,
        )
    }

    fn quote_window(
        mut self,
        min_quote_validity_seconds: u32,
        settlement_buffer_seconds: u32,
    ) -> Self {
        let after = |seconds: u32| QuoteExpiry::after(Duration::from_secs(u64::from(seconds)));
        self.min_valid_until = after(min_quote_validity_seconds);
        self.buffer_end = after(settlement_buffer_seconds);
        self
    }

    pub const fn nonce(mut self, nonce: Nonce) -> Self {
        self.nonce = Some(nonce);
        self
    }

    /// Maker owner in the signed preimage, required even when it signs itself.
    pub const fn maker_owner(mut self, owner: crate::Address32) -> Self {
        self.maker = Some(*owner.as_bytes());
        self
    }

    fn resolve(&self) -> Result<Resolved, QuoteBuildError> {
        if !self.binding.offered_terms.contains(&self.terms) {
            return Err(QuoteBuildError::TermsNotOffered {
                requested: self.terms,
                offered: self.binding.offered_terms.clone(),
            });
        }
        let valid_until = self
            .valid_until
            .ok_or(QuoteBuildError::Missing("valid_until"))?;
        let min_valid_until = self
            .min_valid_until
            .ok_or(QuoteBuildError::Missing("min_valid_until"))?;
        if valid_until > self.binding.market_expiry {
            return Err(QuoteBuildError::ValidUntilAfterMarketExpiry {
                valid_until,
                market_expiry: self.binding.market_expiry,
            });
        }
        if valid_until < min_valid_until {
            return Err(QuoteBuildError::ValidUntilTooSoon {
                valid_until,
                min_valid_until,
            });
        }
        if let Some(buffer_end) = self.buffer_end
            && valid_until <= buffer_end
        {
            return Err(QuoteBuildError::ValidUntilWithinSettlementBuffer {
                valid_until,
                buffer_end,
            });
        }
        Ok(Resolved {
            price: self.price.ok_or(QuoteBuildError::Missing("price"))?,
            valid_until,
            nonce: self.nonce.ok_or(QuoteBuildError::Missing("nonce"))?,
            maker: self.maker.ok_or(QuoteBuildError::Missing("maker_owner"))?,
        })
    }

    fn args_for(&self, resolved: Resolved) -> OrderPreimageArgs {
        OrderPreimageArgs {
            chain_id: self.binding.chain_id,
            program_id: self.binding.program_id,
            side: self.binding.side,
            position_type: self.terms.position_type(),
            market: self.binding.market,
            strike: self.terms.strike().value(),
            quantity: self.binding.quantity.value(),
            gross_price: resolved.price.value(),
            valid_until: resolved.valid_until.as_unix_seconds(),
            maker: resolved.maker,
            taker: self.binding.taker,
            nonce: resolved.nonce.value(),
        }
    }

    /// The exact preimage the server will reconstruct for this quote.
    ///
    /// # Errors
    /// See [`QuoteBuildError`].
    pub fn preimage_args(&self) -> Result<OrderPreimageArgs, QuoteBuildError> {
        if self.terms.is_spread() {
            return Err(crate::orders::OrderError::SpreadLayoutRequired.into());
        }
        Ok(self.args_for(self.resolve()?))
    }

    /// Sign with a local key.
    ///
    /// Reusable builder. Give each quote a fresh nonce to avoid duplicate order IDs.
    ///
    /// # Errors
    /// See [`QuoteBuildError`].
    pub fn sign<S: SignerLike + ?Sized>(
        &self,
        signer: &S,
    ) -> Result<QuoteMessage, QuoteBuildError> {
        let (resolved, order_id, signature) = self.sign_parts(signer)?;
        Ok(self.assemble(resolved, order_id, signature))
    }

    /// Sign with a remote or hardware signer.
    ///
    /// The returned signature is verified against the signer's own public key
    /// before the quote is assembled.
    ///
    /// # Errors
    /// See [`QuoteBuildError`].
    pub async fn sign_with_async_signer<S: AsyncSignerLike + ?Sized>(
        &self,
        signer: &S,
    ) -> Result<QuoteMessage, QuoteBuildError> {
        let resolved = self.resolve()?;
        let order_id = self.order_id(resolved)?;
        let signature = crate::orders::sign_order_id_with_async_signer(&order_id, signer).await?;
        Ok(self.assemble(resolved, order_id, signature))
    }

    fn sign_parts<S: SignerLike + ?Sized>(
        &self,
        signer: &S,
    ) -> Result<(Resolved, [u8; 32], [u8; 64]), QuoteBuildError> {
        let resolved = self.resolve()?;
        let order_id = self.order_id(resolved)?;
        Ok((
            resolved,
            order_id,
            sign_order_id_with_signer(&order_id, signer),
        ))
    }

    pub fn spread_preimage_args(
        &self,
    ) -> Result<crate::orders::SpreadOrderPreimageArgs, QuoteBuildError> {
        self.spread_args_for(self.resolve()?)
    }

    fn spread_args_for(
        &self,
        resolved: Resolved,
    ) -> Result<crate::orders::SpreadOrderPreimageArgs, QuoteBuildError> {
        let args = self.args_for(resolved);
        Ok(crate::orders::SpreadOrderPreimageArgs {
            chain_id: args.chain_id,
            program_id: args.program_id,
            side: args.side,
            position_type: crate::spreads::SpreadType::try_from(u8::from(
                self.terms.to_chain().position_type(),
            ))?,
            market: args.market,
            lower_strike: self.terms.to_chain().strike().value(),
            upper_strike: self
                .terms
                .upper_strike()
                .ok_or(QuoteBuildError::MarketBinding("spread terms"))?
                .value(),
            quantity: args.quantity,
            gross_price: args.gross_price,
            valid_until: args.valid_until,
            maker: args.maker,
            taker: args.taker,
            nonce: args.nonce,
        })
    }

    fn order_id(&self, resolved: Resolved) -> Result<[u8; 32], QuoteBuildError> {
        if self.terms.is_spread() {
            Ok(crate::orders::compute_spread_order_id(
                &self.spread_args_for(resolved)?,
            )?)
        } else {
            Ok(compute_order_id(&self.args_for(resolved))?)
        }
    }

    fn assemble(
        &self,
        resolved: Resolved,
        order_id: [u8; 32],
        signature: [u8; 64],
    ) -> QuoteMessage {
        QuoteMessage {
            rfq_id: self.binding.rfq_id,
            terms: self.terms,
            side: self.binding.side,
            price: resolved.price,
            valid_until: resolved.valid_until,
            nonce: resolved.nonce,
            order_id: OrderId::new(order_id),
            signature: encode_base58(&signature),
        }
    }
}

impl QuoteMessage {
    /// Add the replaced order's ID without signing again; supports remote signers.
    #[must_use]
    pub fn into_replacement(self, old_order_id: OrderId) -> ReplaceQuoteMessage {
        ReplaceQuoteMessage {
            old_order_id,
            rfq_id: self.rfq_id,
            terms: self.terms,
            side: self.side,
            price: self.price,
            valid_until: self.valid_until,
            nonce: self.nonce,
            order_id: self.order_id,
            signature: self.signature,
        }
    }
}

#[cfg(test)]
#[path = "quote_tests.rs"]
mod tests;
