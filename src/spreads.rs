//! Cash spreads use quote atoms for collateral and payoff; prices use 1e9.

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SpreadType {
    CallSpread = 2,
    PutSpread = 3,
}

impl TryFrom<u8> for SpreadType {
    type Error = SpreadError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            2 => Ok(Self::CallSpread),
            3 => Ok(Self::PutSpread),
            _ => Err(SpreadError::InvalidType),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SpreadError {
    #[error("taker_buy_unsupported")]
    TakerBuyUnsupported,
    #[error("expected CallSpread or PutSpread")]
    InvalidType,
    #[error("spread requires lower < upper")]
    InvalidStrikes,
    #[error("spread requires nonzero quantity and 0 < premium < width")]
    InvalidPremium,
    #[error("spread amount overflow or unsupported decimals (expected 0..=18)")]
    Overflow,
    #[error("invalid spread fee basis points")]
    InvalidFee,
    #[error("collateral is below the required liability")]
    InsufficientCollateral,
    #[error("invalid pricing inputs or long-leg values")]
    InvalidPricing,
}

pub fn width(lower: u64, upper: u64) -> Result<u64, SpreadError> {
    upper
        .checked_sub(lower)
        .filter(|v| *v > 0)
        .ok_or(SpreadError::InvalidStrikes)
}

/// Put spreads allow a zero lower strike; calls require a positive lower strike.
pub fn validate_strikes(kind: SpreadType, lower: u64, upper: u64) -> Result<u64, SpreadError> {
    if kind == SpreadType::CallSpread && lower == 0 {
        return Err(SpreadError::InvalidStrikes);
    }
    width(lower, upper)
}

pub fn validate_premium(
    kind: SpreadType,
    lower: u64,
    upper: u64,
    quantity: u64,
    price: u64,
) -> Result<(), SpreadError> {
    let width = validate_strikes(kind, lower, upper)?;
    if quantity == 0 || price == 0 || price >= width {
        return Err(SpreadError::InvalidPremium);
    }
    Ok(())
}

pub fn quote_amount(
    price: u64,
    quantity: u64,
    quote_decimals: u8,
    underlying_decimals: u8,
    ceil: bool,
) -> Result<u64, SpreadError> {
    if quote_decimals > 18 || underlying_decimals > 18 {
        return Err(SpreadError::Overflow);
    }
    let quote_scale = 10u128.pow(u32::from(quote_decimals));
    let denominator = 1_000_000_000u128 * 10u128.pow(u32::from(underlying_decimals));
    let numerator = u128::from(price) * u128::from(quantity);
    let result = if quote_scale <= denominator {
        let divisor = denominator / quote_scale;
        numerator / divisor + u128::from(ceil && numerator % divisor != 0)
    } else {
        numerator
            .checked_mul(quote_scale / denominator)
            .ok_or(SpreadError::Overflow)?
    };
    u64::try_from(result).map_err(|_| SpreadError::Overflow)
}

pub fn collateral(
    lower: u64,
    upper: u64,
    quantity: u64,
    quote_decimals: u8,
    underlying_decimals: u8,
) -> Result<u64, SpreadError> {
    quote_amount(
        width(lower, upper)?,
        quantity,
        quote_decimals,
        underlying_decimals,
        true,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpreadTerms {
    pub kind: SpreadType,
    pub lower_strike: u64,
    pub upper_strike: u64,
    pub quantity: u64,
    pub quote_decimals: u8,
    pub underlying_decimals: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpreadPremium {
    pub collateral: u64,
    pub gross_premium: u64,
    pub protocol_fee: u64,
    pub net_premium: u64,
    pub prefunded_quote: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpreadSettlement {
    pub maker_payoff: u64,
    pub taker_return: u64,
}

impl SpreadTerms {
    pub fn collateral(self) -> Result<u64, SpreadError> {
        collateral(
            self.lower_strike,
            self.upper_strike,
            self.quantity,
            self.quote_decimals,
            self.underlying_decimals,
        )
    }

    pub fn payoff(self, settlement_price: u64) -> Result<u64, SpreadError> {
        let width = width(self.lower_strike, self.upper_strike)?;
        let intrinsic = match self.kind {
            SpreadType::CallSpread => settlement_price.saturating_sub(self.lower_strike),
            SpreadType::PutSpread => self.upper_strike.saturating_sub(settlement_price),
        }
        .min(width);
        self.quote(intrinsic, true)
    }

    pub fn settle(
        self,
        settlement_price: u64,
        actual_collateral: u64,
    ) -> Result<SpreadSettlement, SpreadError> {
        if actual_collateral < self.collateral()? {
            return Err(SpreadError::InsufficientCollateral);
        }
        let maker_payoff = self.payoff(settlement_price)?;
        let taker_return = actual_collateral
            .checked_sub(maker_payoff)
            .ok_or(SpreadError::InsufficientCollateral)?;
        Ok(SpreadSettlement {
            maker_payoff,
            taker_return,
        })
    }

    pub fn premium(
        self,
        price: u64,
        premium_bps: u16,
        volume_bps: u16,
    ) -> Result<SpreadPremium, SpreadError> {
        validate_premium(
            self.kind,
            self.lower_strike,
            self.upper_strike,
            self.quantity,
            price,
        )?;
        if premium_bps > 10_000 || volume_bps > 10_000 {
            return Err(SpreadError::InvalidFee);
        }
        let collateral = self.collateral()?;
        let gross_premium = self.quote(price, false)?;
        let exposure = self.quote(width(self.lower_strike, self.upper_strike)?, false)?;
        if gross_premium == 0 || gross_premium >= collateral || exposure == 0 {
            return Err(SpreadError::InvalidPremium);
        }
        let fee = |amount: u64, bps: u16| -> Result<u64, SpreadError> {
            u64::try_from(u128::from(amount) * u128::from(bps) / 10_000)
                .map_err(|_| SpreadError::Overflow)
        };
        let premium_fee = gross_premium - self.quote(price - fee(price, premium_bps)?, false)?;
        let protocol_fee = premium_fee.min(fee(exposure, volume_bps)?);
        let net_premium = gross_premium - protocol_fee;
        Ok(SpreadPremium {
            collateral,
            gross_premium,
            protocol_fee,
            net_premium,
            prefunded_quote: collateral - net_premium,
        })
    }

    fn quote(self, price: u64, ceil: bool) -> Result<u64, SpreadError> {
        quote_amount(
            price,
            self.quantity,
            self.quote_decimals,
            self.underlying_decimals,
            ceil,
        )
    }
}

/// Both long legs are valued by the caller's model with one expiry/volatility context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpreadPricingContext {
    pub time_to_expiry_years: f64,
    pub volatility: f64,
}

pub fn price_spread(
    kind: SpreadType,
    lower_strike: f64,
    upper_strike: f64,
    context: SpreadPricingContext,
    mut long_leg_value: impl FnMut(bool, f64, SpreadPricingContext) -> f64,
) -> Result<f64, SpreadError> {
    if !lower_strike.is_finite()
        || !upper_strike.is_finite()
        || lower_strike < 0.0
        || (kind == SpreadType::CallSpread && lower_strike == 0.0)
        || upper_strike <= lower_strike
        || !context.time_to_expiry_years.is_finite()
        || context.time_to_expiry_years < 0.0
        || !context.volatility.is_finite()
        || context.volatility < 0.0
    {
        return Err(SpreadError::InvalidPricing);
    }
    let is_put = kind == SpreadType::PutSpread;
    let low = if is_put && lower_strike == 0.0 {
        0.0
    } else {
        long_leg_value(is_put, lower_strike, context)
    };
    let high = long_leg_value(is_put, upper_strike, context);
    let price = if is_put { high - low } else { low - high };
    if !low.is_finite()
        || !high.is_finite()
        || low < 0.0
        || high < 0.0
        || !price.is_finite()
        || price < 0.0
        || price > upper_strike - lower_strike
    {
        return Err(SpreadError::InvalidPricing);
    }
    Ok(price)
}
