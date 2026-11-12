use serde::{Deserialize, Serialize};

use super::{
    CapError, ClaimErrorReason, Delivery, InviteErrorReason, QuoteLockedReason, RateLimitReason,
    ServerError, WsErrorCode,
};

/// What a client does next after an error; the WS error catalog lists it per code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorAction {
    /// Transient: send the same request again after a backoff.
    Retry,
    /// Authenticate again, then resend.
    Reauthenticate,
    /// The input went stale: fetch fresh state and build a new request.
    Refresh,
    /// The command may already have taken effect: look up its status before anything else.
    Reconcile,
    /// The request is invalid: change it before sending again.
    FixRequest,
    /// Nothing the client resends succeeds until the venue, market or account state changes.
    Stop,
}

impl ErrorAction {
    /// The same request may be sent again unchanged.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Retry | Self::Reauthenticate)
    }

    /// A submission with `unknown` delivery may still execute, so any action becomes `Reconcile`.
    #[must_use]
    pub const fn for_delivery(self, delivery: Delivery) -> Self {
        match delivery {
            Delivery::Unknown => Self::Reconcile,
            Delivery::NotForwarded | Delivery::NotApplicable => self,
        }
    }
}

impl WsErrorCode {
    #[must_use]
    pub const fn action(self) -> ErrorAction {
        use ErrorAction::{FixRequest, Reauthenticate, Reconcile, Refresh, Retry, Stop};
        match self {
            Self::RateLimited
            | Self::ServerOverloaded
            | Self::ServerBusy
            | Self::InternalError
            | Self::TemporarilyUnavailable
            | Self::ClockError
            | Self::HelloTimeout
            | Self::TooManyAuthAttempts
            | Self::VaultAuthorizationUnavailable
            | Self::LadderStrikeSourceUnavailable
            | Self::ResourceLimit
            | Self::TxBuildFailed
            | Self::SponsoredTxNotReady
            | Self::VaultProjectionUnavailable
            | Self::VaultSwapOracleStale
            | Self::VaultSwapRouteUnavailable
            | Self::VaultSwapServiceUnavailable => Retry,

            Self::SessionExpired
            | Self::SessionConflict
            | Self::AuthTimeout
            | Self::VaultAuthRequired
            | Self::VaultSessionInvalidated
            | Self::VaultAuthorizationFailed => Reauthenticate,

            Self::RfqClosed
            | Self::QuoteRefreshRequired
            | Self::CapsAuthorityUnavailable
            | Self::VaultSwapQuoteExpired
            | Self::VaultSwapSignedTxRejected => Refresh,

            Self::DuplicateRfqId
            | Self::DuplicateQuote
            | Self::KernelTimeout
            | Self::OrderAlreadySubmitted => Reconcile,

            Self::ParseError
            | Self::TooManyParseErrors
            | Self::HelloRequired
            | Self::HelloAlreadySent
            | Self::TimeOverflow
            | Self::BatchQuotesTooLarge
            | Self::AlreadyAuthenticated
            | Self::ChallengeNotIssued
            | Self::ChallengeMismatch
            | Self::PubkeyMismatch
            | Self::InvalidSignature
            | Self::OperatorMismatch
            | Self::VaultAuthNotStarted
            | Self::VaultResumeNotSupported
            | Self::InvalidVaultSubscription
            | Self::InvalidProducts
            | Self::SubscriptionScopeTooLarge
            | Self::InvalidLimit
            | Self::InvalidCursor
            | Self::InvalidTimeout
            | Self::MarketNotFound
            | Self::InvalidStrikeNotional
            | Self::InsufficientCollateral
            | Self::InvalidQuantitySizeRule
            | Self::ProductNotAllowed
            | Self::InvalidStrikes
            | Self::TakerBuyUnsupported
            | Self::SpreadInvalidStrikes
            | Self::SpreadTickViolation
            | Self::SpreadPremiumInvalid
            | Self::QuoteIdentityMismatch
            | Self::InvalidVaultCounterparty
            | Self::InvalidSignedTx
            | Self::InvalidAmount
            | Self::VaultMismatch
            | Self::VaultSwapInvalidAmount
            | Self::VaultSwapIdempotencyConflict => FixRequest,

            Self::TradingPaused
            | Self::SessionReplaced
            | Self::SessionLimitExceeded
            | Self::Blacklisted
            | Self::MakerUnregistered
            | Self::MakerNotRegistered
            | Self::VaultOperatorUnauthorized
            | Self::HistoryProjectionIncomplete
            | Self::InvalidProgramId
            | Self::UnsupportedProtocolVersion
            | Self::MissingSizeRuleForUnderlyingMint
            | Self::InvalidUnderlyingMintKey
            | Self::MissingTickForUnderlyingMint
            | Self::MarketFinalized
            | Self::MarketDisabled
            | Self::MarketExpired
            | Self::MarketPreExpiry
            | Self::VaultNotTradable
            | Self::VaultSafetyMode
            | Self::VaultPendingCapital
            | Self::VaultTradingClosed
            | Self::VaultGovernanceRequired
            | Self::VaultSwapInvalidState => Stop,
        }
    }
}

impl ServerError {
    #[must_use]
    pub const fn action(&self) -> ErrorAction {
        use ErrorAction::{FixRequest, Reauthenticate, Reconcile, Refresh, Retry, Stop};
        match self {
            Self::Generic { code, .. } => code.action(),
            Self::Unauthenticated { .. } => Reauthenticate,
            Self::QuoteExpired | Self::SignatureTimeout => Refresh,
            Self::QuoteLocked { reason } => match reason {
                QuoteLockedReason::OrderSubmitted => Reconcile,
                QuoteLockedReason::RfqLocked => Refresh,
            },
            Self::OracleNotReady
            | Self::OraclePriceStale { .. }
            | Self::OraclePriceNotReady
            | Self::MarketMetadataIncomplete { .. }
            | Self::TokenMetadataIncomplete { .. }
            | Self::InternalError
            | Self::KernelNotAvailable
            | Self::ServerShuttingDown => Retry,
            Self::RateLimit(reason) => match reason {
                RateLimitReason::TooManyActiveRfqsPerTaker
                | RateLimitReason::TooManyActiveRfqsTotal => Retry,
                RateLimitReason::TooManyFailedOrdersPerRfq => Reconcile,
                RateLimitReason::TooManyQuotesPerRfq | RateLimitReason::TooManySessionsPerUser => {
                    Stop
                }
            },
            Self::Cap(cap) => match cap {
                CapError::CapsUnavailable => Retry,
                CapError::QuoteLiabilityCapExceeded { .. }
                | CapError::TokenOiCapExceeded { .. }
                | CapError::MarketOiCapExceeded { .. }
                | CapError::MakerPositionCapExceeded { .. }
                | CapError::MakerNotionalCapExceeded { .. }
                | CapError::MakerInsufficientBalance { .. }
                | CapError::QuoteNotionalCapExceeded { .. }
                | CapError::MakerQuoteNotionalCapExceeded { .. }
                | CapError::MakerQuoteLiabilityCapExceeded { .. } => FixRequest,
            },
            Self::Invite(reason) => match reason {
                InviteErrorReason::InternalError => Retry,
                InviteErrorReason::InvalidCode => FixRequest,
                InviteErrorReason::CodeExhausted
                | InviteErrorReason::CodeExpired
                | InviteErrorReason::CodeDisabled
                | InviteErrorReason::CodeOwnerInactive
                | InviteErrorReason::CodeOwnerBlacklisted
                | InviteErrorReason::AlreadyRegistered => Stop,
            },
            Self::Claim(reason) => match reason {
                ClaimErrorReason::InternalError => Retry,
                ClaimErrorReason::InvalidFormat
                | ClaimErrorReason::CodeTaken
                | ClaimErrorReason::Reserved => FixRequest,
                ClaimErrorReason::NotRegistered => Stop,
            },
            Self::QuoteNotFound | Self::UnknownOrder => Refresh,
            Self::QuoteExpiryTooShort { .. }
            | Self::InvalidStrike
            | Self::InvalidValidUntil
            | Self::OrderIdMismatch
            | Self::InvalidPositionType
            | Self::InvalidMarket { .. }
            | Self::WrongEndpoint { .. } => FixRequest,
            Self::RfqAlreadyLocked => Reconcile,
            Self::RfqNotFound | Self::RfqNotActive => Refresh,
            Self::Unauthorized { .. }
            | Self::InvalidState { .. }
            | Self::DbDisabled { .. }
            | Self::InviteRequired => Stop,
        }
    }
}
