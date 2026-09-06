//! Correct-by-construction quote assembly.
//!
//! A quote is two things that must agree exactly: the signed order preimage and
//! the [`QuoteMessage`] that carries it. They repeat `strike`, `price`,
//! `valid_until` and `nonce`, and the `order_id` must hash precisely those
//! values. Filling both by hand makes every field an opportunity for a silent
//! `order_id preimage mismatch` rejection.
//!
//! [`RfqBinding`] decodes an RFQ's identities once and resolves its strike set;
//! [`QuoteBuilder`] then derives the preimage and the message from a single
//! resolved set of values, so the two cannot disagree.

use crate::orders::{OrderPreimageArgs, SignerLike, compute_order_id, sign_order_id_with_signer};
use crate::signing::{AsyncSignerLike, SigningError};
use crate::types::ids::{Nonce, OrderId, Price, Quantity, Strike};
use crate::types::{PositionType, QuoteExpiry};
use crate::wire::{WireError, decode_base58_32, encode_base58};
use crate::ws::types::{QuoteMessage, ReplaceQuoteMessage, RfqBroadcastMessage};
use uuid::Uuid;

/// A quote could not be assembled from the RFQ and the supplied values.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum QuoteBuildError {
    #[error("rfq field `{field}` is not a valid base58 32-byte key: {source}")]
    Identity {
        field: &'static str,
        #[source]
        source: WireError,
    },

    /// The engine only admits quotes on strikes the RFQ asked for
    /// (`RfqBroadcast.strike` plus `order_options`), so an unlisted strike is
    /// rejected here rather than after a round trip.
    #[error("strike {requested} is not offered by this rfq; offered: {offered:?}")]
    StrikeNotOffered {
        requested: Strike,
        offered: Vec<Strike>,
    },

    #[error("quote is missing `{0}`")]
    Missing(&'static str),

    #[error("signing failed: {0}")]
    Signing(#[from] SigningError),
}

/// An RFQ with its identities decoded and its strike set resolved.
///
/// Build one per RFQ and reuse it for every quote and replacement on that RFQ:
/// the base58 decoding and strike resolution happen once.
#[derive(Debug, Clone)]
pub struct RfqBinding {
    rfq_id: Uuid,
    chain_id: u64,
    program_id: [u8; 32],
    market: [u8; 32],
    taker: [u8; 32],
    position_type: PositionType,
    quantity: Quantity,
    requested_strike: Strike,
    offered_strikes: Vec<Strike>,
}

fn identity(field: &'static str, value: &str) -> Result<[u8; 32], QuoteBuildError> {
    decode_base58_32(value).map_err(|source| QuoteBuildError::Identity { field, source })
}

impl RfqBinding {
    /// Decode and validate an inbound [`RfqBroadcastMessage`].
    ///
    /// # Errors
    /// Returns [`QuoteBuildError::Identity`] if the program, market or taker key
    /// is not valid base58.
    pub fn from_broadcast(rfq: &RfqBroadcastMessage) -> Result<Self, QuoteBuildError> {
        let mut offered_strikes = Vec::with_capacity(1 + rfq.order_options.len());
        offered_strikes.push(rfq.strike);
        offered_strikes.extend(
            rfq.order_options
                .iter()
                .map(|option| option.strike)
                .filter(|strike| *strike != rfq.strike),
        );

        Ok(Self {
            rfq_id: rfq.rfq_id,
            chain_id: rfq.market.chain_id.value(),
            program_id: identity("market.program_id", &rfq.market.program_id)?,
            market: identity("market.market_pda", &rfq.market.market_pda)?,
            taker: identity("taker", &rfq.taker)?,
            position_type: rfq.position_type,
            quantity: rfq.quantity,
            requested_strike: rfq.strike,
            offered_strikes,
        })
    }

    #[must_use]
    pub const fn rfq_id(&self) -> Uuid {
        self.rfq_id
    }

    /// The size the engine prices this quote against. It comes from the RFQ, not
    /// from the maker, but it is part of the signed preimage.
    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    #[must_use]
    pub const fn position_type(&self) -> PositionType {
        self.position_type
    }

    /// Every strike this RFQ will accept a quote on.
    #[must_use]
    pub fn offered_strikes(&self) -> &[Strike] {
        &self.offered_strikes
    }

    /// Start a quote on this RFQ's requested strike.
    pub const fn quote(&self) -> QuoteBuilder<'_> {
        QuoteBuilder {
            binding: self,
            strike: self.requested_strike,
            price: None,
            valid_until: None,
            nonce: None,
            maker: None,
        }
    }
}

/// The maker-supplied half of a quote, after validation.
#[derive(Debug, Clone, Copy)]
struct Resolved {
    strike: Strike,
    price: Price,
    valid_until: QuoteExpiry,
    nonce: Nonce,
}

/// Accumulates the maker's side of a quote, then signs it.
#[derive(Debug, Clone)]
#[must_use = "a builder does nothing until it is signed"]
pub struct QuoteBuilder<'a> {
    binding: &'a RfqBinding,
    strike: Strike,
    price: Option<Price>,
    valid_until: Option<QuoteExpiry>,
    nonce: Option<Nonce>,
    maker: Option<[u8; 32]>,
}

impl QuoteBuilder<'_> {
    /// Quote an alternative strike offered by the RFQ instead of its requested one.
    pub const fn strike(mut self, strike: Strike) -> Self {
        self.strike = strike;
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

    pub const fn nonce(mut self, nonce: Nonce) -> Self {
        self.nonce = Some(nonce);
        self
    }

    /// The maker identity signed into the order preimage.
    ///
    /// The server reconstructs the preimage with the session's authenticated
    /// maker owner. When quotes are signed by a delegated key, set the owner
    /// here or every quote is rejected as `order_id preimage mismatch`.
    /// Defaults to the signing key's own public key.
    pub const fn maker_owner(mut self, owner: [u8; 32]) -> Self {
        self.maker = Some(owner);
        self
    }

    fn resolve(&self) -> Result<Resolved, QuoteBuildError> {
        if !self.binding.offered_strikes.contains(&self.strike) {
            return Err(QuoteBuildError::StrikeNotOffered {
                requested: self.strike,
                offered: self.binding.offered_strikes.clone(),
            });
        }
        Ok(Resolved {
            strike: self.strike,
            price: self.price.ok_or(QuoteBuildError::Missing("price"))?,
            valid_until: self
                .valid_until
                .ok_or(QuoteBuildError::Missing("valid_until"))?,
            nonce: self.nonce.ok_or(QuoteBuildError::Missing("nonce"))?,
        })
    }

    const fn maker_for(&self, signer_pubkey: [u8; 32]) -> [u8; 32] {
        match self.maker {
            Some(owner) => owner,
            None => signer_pubkey,
        }
    }

    fn args_for(&self, resolved: Resolved, maker: [u8; 32]) -> OrderPreimageArgs {
        OrderPreimageArgs {
            chain_id: self.binding.chain_id,
            program_id: self.binding.program_id,
            // Maker-quoted RFQs are always sells from the maker's side.
            is_taker_buy: false,
            position_type: self.binding.position_type,
            market: self.binding.market,
            strike: resolved.strike.value(),
            quantity: self.binding.quantity.value(),
            gross_price: resolved.price.value(),
            valid_until: resolved.valid_until.as_unix_seconds(),
            maker,
            taker: self.binding.taker,
            nonce: resolved.nonce.value(),
        }
    }

    /// The exact preimage the server will reconstruct for this quote.
    ///
    /// Exposed so a maker can inspect the bytes or sign them out of band.
    /// `maker` is the signing key's fallback identity; an owner set with
    /// [`Self::maker_owner`] wins, matching the `sign` paths.
    ///
    /// # Errors
    /// See [`QuoteBuildError`].
    pub fn preimage_args(&self, maker: [u8; 32]) -> Result<OrderPreimageArgs, QuoteBuildError> {
        let resolved = self.resolve()?;
        Ok(self.args_for(resolved, self.maker_for(maker)))
    }

    /// Sign with a local key.
    ///
    /// The builder is borrowed, so it can be signed more than once — useful for
    /// quoting several strikes off one binding. Give each quote its own nonce:
    /// signing twice with the same one reproduces the same `order_id`, which
    /// the server rejects as a duplicate.
    ///
    /// # Errors
    /// See [`QuoteBuildError`].
    pub fn sign<S: SignerLike>(&self, signer: &S) -> Result<QuoteMessage, QuoteBuildError> {
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
        let args = self.args_for(resolved, self.maker_for(signer.pubkey_bytes()));
        let order_id = compute_order_id(&args);
        let signature = crate::orders::sign_order_id_with_async_signer(&order_id, signer).await?;
        Ok(self.assemble(resolved, order_id, signature))
    }

    fn sign_parts<S: SignerLike>(
        &self,
        signer: &S,
    ) -> Result<(Resolved, [u8; 32], [u8; 64]), QuoteBuildError> {
        let resolved = self.resolve()?;
        let args = self.args_for(resolved, self.maker_for(signer.pubkey_bytes()));
        let order_id = compute_order_id(&args);
        Ok((
            resolved,
            order_id,
            sign_order_id_with_signer(&order_id, signer),
        ))
    }

    /// Assemble the message from the same `resolved` values the preimage used.
    /// This is the only place a `QuoteMessage` is built, so the wire fields and
    /// the signed fields are the same values by construction.
    fn assemble(
        &self,
        resolved: Resolved,
        order_id: [u8; 32],
        signature: [u8; 64],
    ) -> QuoteMessage {
        QuoteMessage {
            rfq_id: self.binding.rfq_id,
            strike: resolved.strike,
            price: resolved.price,
            valid_until: resolved.valid_until,
            nonce: resolved.nonce,
            order_id: OrderId::new(order_id),
            signature: encode_base58(&signature),
        }
    }
}

impl QuoteMessage {
    /// Turn this signed quote into a replacement for an earlier order.
    ///
    /// A replacement is the same signed quote plus the order it supersedes, so
    /// it is a conversion rather than a second signing path — which keeps it
    /// available to remote signers too.
    #[must_use]
    pub fn into_replacement(self, old_order_id: OrderId) -> ReplaceQuoteMessage {
        ReplaceQuoteMessage {
            old_order_id,
            rfq_id: self.rfq_id,
            strike: self.strike,
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
