//! Quote assembly: [`RfqBinding`] decodes identities and resolves offered strikes;
//! [`QuoteBuilder`] derives the signed preimage and [`QuoteMessage`] from the
//! same strike, price, expiry and nonce.

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

    #[error("rfq market does not match trusted signing terms: {0}")]
    MarketBinding(&'static str),

    /// Strike absent from `RfqBroadcast.strike` and `order_options`.
    #[error("strike {requested} is not offered by this rfq; offered: {offered:?}")]
    StrikeNotOffered {
        requested: Strike,
        offered: Vec<Strike>,
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

    #[error("signing failed: {0}")]
    Signing(#[from] SigningError),
}

/// An RFQ with its identities decoded and its strike set resolved.
///
/// Reuse per RFQ to avoid repeated decoding and strike resolution.
#[derive(Debug, Clone)]
pub struct RfqBinding {
    rfq_id: Uuid,
    chain_id: u64,
    program_id: [u8; 32],
    market: [u8; 32],
    market_expiry: QuoteExpiry,
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
        let (market, market_expiry) = validate_market_binding(rfq, expected_program)?;
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
            program_id: expected_program,
            market,
            market_expiry,
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

    /// RFQ quantity used for pricing and included in the signed preimage.
    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    #[must_use]
    pub const fn position_type(&self) -> PositionType {
        self.position_type
    }

    /// Latest `valid_until` the venue accepts for this RFQ.
    #[must_use]
    pub const fn market_expiry(&self) -> QuoteExpiry {
        self.market_expiry
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
            min_valid_until: None,
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
    if identity("market.program_id", &descriptor.program_id)? != expected_program {
        return Err(MarketBinding("program_id"));
    }
    if descriptor.is_put != (rfq.position_type == PositionType::CashSecuredPut) {
        return Err(MarketBinding("position_type"));
    }
    let underlying = identity("market.underlying_mint", &descriptor.underlying_mint)?;
    let quote = identity("market.quote_mint", &descriptor.quote_mint)?;
    let (collateral, settlement) = if descriptor.is_put {
        (quote, underlying)
    } else {
        (underlying, quote)
    };
    if identity("market.collateral_mint", &descriptor.collateral_mint)? != collateral
        || identity("market.settlement_mint", &descriptor.settlement_mint)? != settlement
    {
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
    let market = identity("market.market_pda", &descriptor.market_pda)?;
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
    strike: Strike,
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
    strike: Strike,
    price: Option<Price>,
    valid_until: Option<QuoteExpiry>,
    min_valid_until: Option<QuoteExpiry>,
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

    /// Earliest `valid_until` the venue accepts: now plus `min_quote_validity_seconds`
    /// from `GET /api/v1/venue`.
    pub const fn min_valid_until(mut self, min_valid_until: QuoteExpiry) -> Self {
        self.min_valid_until = Some(min_valid_until);
        self
    }

    pub const fn nonce(mut self, nonce: Nonce) -> Self {
        self.nonce = Some(nonce);
        self
    }

    /// Maker owner in the signed preimage, required even when it signs itself.
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
        Ok(Resolved {
            strike: self.strike,
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
            is_taker_buy: false,
            position_type: self.binding.position_type,
            market: self.binding.market,
            strike: resolved.strike.value(),
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
        let order_id = compute_order_id(&self.args_for(resolved));
        let signature = crate::orders::sign_order_id_with_async_signer(&order_id, signer).await?;
        Ok(self.assemble(resolved, order_id, signature))
    }

    fn sign_parts<S: SignerLike + ?Sized>(
        &self,
        signer: &S,
    ) -> Result<(Resolved, [u8; 32], [u8; 64]), QuoteBuildError> {
        let resolved = self.resolve()?;
        let order_id = compute_order_id(&self.args_for(resolved));
        Ok((
            resolved,
            order_id,
            sign_order_id_with_signer(&order_id, signer),
        ))
    }

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
    /// Add the replaced order's ID without signing again; supports remote signers.
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
