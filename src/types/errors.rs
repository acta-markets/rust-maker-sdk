use super::ids::{Balance, Quantity};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
// Variants share suffix to match server-side rate limit naming convention.
#[allow(clippy::enum_variant_names)]
pub enum RateLimitReason {
    TooManyActiveRfqsPerTaker,
    TooManyActiveRfqsTotal,
    TooManyQuotesPerRfq,
    TooManySessionsPerUser,
}

impl std::fmt::Display for RateLimitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::TooManyActiveRfqsPerTaker => "Too many active RFQs for your account",
            Self::TooManyActiveRfqsTotal => "System capacity reached, please try again later",
            Self::TooManyQuotesPerRfq => "Too many quotes for this RFQ",
            Self::TooManySessionsPerUser => "Too many active sessions for your account",
        };
        f.write_str(msg)
    }
}

/// Cap violation: protocol-level exposure limits.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CapError {
    #[strum(serialize = "token_oi_cap_exceeded")]
    #[error("Token OI cap exceeded for {underlying_mint} ({current}/{limit})")]
    TokenOiCapExceeded {
        underlying_mint: String,
        current: Quantity,
        limit: Quantity,
    },

    #[strum(serialize = "market_oi_cap_exceeded")]
    #[error("Market OI cap exceeded for {market_id} ({current}/{limit})")]
    MarketOiCapExceeded {
        market_id: String,
        current: Quantity,
        limit: Quantity,
    },

    #[strum(serialize = "maker_position_cap_exceeded")]
    #[error("Maker position cap exceeded ({current}/{limit})")]
    MakerPositionCapExceeded { current: u32, limit: u32 },

    #[strum(serialize = "maker_notional_cap_exceeded")]
    #[error("Maker notional cap exceeded for {underlying_mint} ({current}/{limit})")]
    MakerNotionalCapExceeded {
        underlying_mint: String,
        current: Quantity,
        limit: Quantity,
    },

    #[strum(serialize = "maker_insufficient_balance")]
    #[error("Maker insufficient balance ({available} available, {required} required)")]
    MakerInsufficientBalance {
        available: Balance,
        required: Balance,
    },

    #[strum(serialize = "quote_notional_cap_exceeded")]
    #[error("Quote notional cap exceeded for {quote_mint} ({current}/{limit})")]
    QuoteNotionalCapExceeded {
        quote_mint: String,
        current: Balance,
        limit: Balance,
    },

    #[strum(serialize = "maker_quote_notional_cap_exceeded")]
    #[error("Maker quote notional cap exceeded for {quote_mint} ({current}/{limit})")]
    MakerQuoteNotionalCapExceeded {
        quote_mint: String,
        current: Balance,
        limit: Balance,
    },
}

impl CapError {
    #[must_use]
    pub fn code(&self) -> &str {
        self.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum UserRole {
    Maker,
    Taker,
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AuthRequiredAction {
    SubmitQuotes,
    CancelQuotes,
    QueryQuotes,
    CreateRfqs,
    AcceptQuotes,
    SubmitSignedTx,
    CancelRfqs,
    AccessRfq,
    RequestPositions,
    Subscribe,
    Unsubscribe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RfqStateError {
    NotActive,
    NotPendingSignature,
    CannotBeCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum QuoteLockedReason {
    RfqLocked,
    OrderSubmitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum DbFeature {
    MakerPositions,
    MakerMarkets,
    MarketDescriptors,
    Expiries,
    Tokens,
}
