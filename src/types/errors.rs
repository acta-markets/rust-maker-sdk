use super::ids::{Balance, Quantity};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
// Variants share suffix to match server-side rate limit naming convention.
#[allow(clippy::enum_variant_names)]
#[non_exhaustive]
pub enum RateLimitReason {
    TooManyActiveRfqsPerTaker,
    TooManyActiveRfqsTotal,
    TooManyQuotesPerRfq,
    TooManyFailedOrdersPerRfq,
    TooManySessionsPerUser,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for RateLimitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::TooManyActiveRfqsPerTaker => "Too many active RFQs for your account",
            Self::TooManyActiveRfqsTotal => "System capacity reached, please try again later",
            Self::TooManyQuotesPerRfq => "Too many quotes for this RFQ",
            Self::TooManyFailedOrdersPerRfq => "Too many unresolved failed orders for this RFQ",
            Self::TooManySessionsPerUser => "Too many active sessions for your account",
            Self::Unknown => "Unknown rate limit",
        };
        f.write_str(msg)
    }
}

/// Cap violation: protocol-level exposure limits.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, AsRefStr, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
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

    /// Venue state, not maker state: the caps authority is catching up and
    /// quoting is paused for everyone. Retry, do not de-risk.
    #[strum(serialize = "caps_unavailable")]
    #[error("Caps authority temporarily unavailable; retry shortly")]
    CapsUnavailable,

    /// A cap variant this SDK predates; the raw payload is preserved so a
    /// message carrying it still parses instead of dropping the connection.
    #[serde(untagged)]
    #[strum(serialize = "unknown")]
    #[error("Unrecognized cap error")]
    Unknown(serde_json::Value),
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
#[non_exhaustive]
pub enum UserRole {
    Maker,
    Taker,
    Owner,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum AuthRequiredAction {
    SubmitQuotes,
    CancelQuotes,
    CreateRfqs,
    AcceptQuotes,
    SubmitSignedTx,
    CancelRfqs,
    AccessRfq,
    RequestPositions,
    Subscribe,
    Unsubscribe,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum RfqStateError {
    NotActive,
    NotPendingSignature,
    CannotBeCancelled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum QuoteLockedReason {
    RfqLocked,
    OrderSubmitted,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum DbFeature {
    MakerPositions,
    MakerMarkets,
    MarketDescriptors,
    Expiries,
    Tokens,
    #[serde(other)]
    Unknown,
}
