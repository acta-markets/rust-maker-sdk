use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr};

/// `Generic.code` / `AuthError.reason`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WsErrorCode {
    ParseError,
    TooManyParseErrors,
    HelloRequired,
    HelloTimeout,
    HelloAlreadySent,
    RateLimited,
    ServerOverloaded,
    ServerBusy,
    InternalError,
    TemporarilyUnavailable,
    ClockError,
    TimeOverflow,
    TradingPaused,
    BatchQuotesTooLarge,

    SessionReplaced,
    SessionExpired,
    SessionLimitExceeded,
    SessionConflict,
    Blacklisted,
    MakerUnregistered,
    MakerNotRegistered,
    AuthTimeout,
    TooManyAuthAttempts,
    AlreadyAuthenticated,
    ChallengeNotIssued,
    ChallengeMismatch,

    PubkeyMismatch,
    InvalidSignature,

    OperatorMismatch,

    VaultAuthNotStarted,
    VaultAuthRequired,
    VaultResumeNotSupported,
    VaultOperatorUnauthorized,
    VaultAuthorizationUnavailable,
    VaultSessionInvalidated,

    InvalidVaultSubscription,
    InvalidProducts,
    SubscriptionScopeTooLarge,
    HistoryProjectionIncomplete,
    InvalidLimit,
    InvalidTimeout,
    MarketNotFound,
    InvalidProgramId,
    UnsupportedProtocolVersion,

    InvalidStrikeNotional,
    InsufficientCollateral,
    InvalidQuantitySizeRule,
    ProductNotAllowed,
    MissingSizeRuleForUnderlyingMint,
    InvalidUnderlyingMintKey,
    MissingTickForUnderlyingMint,
    LadderStrikeSourceUnavailable,
    InvalidStrikes,
    TakerBuyUnsupported,
    SpreadInvalidStrikes,
    SpreadTickViolation,
    SpreadPremiumInvalid,
    DuplicateRfqId,
    RfqClosed,
    MarketFinalized,
    MarketDisabled,
    MarketExpired,
    MarketPreExpiry,
    QuoteIdentityMismatch,
    QuoteRefreshRequired,
    DuplicateQuote,
    ResourceLimit,
    KernelTimeout,

    CapsAuthorityUnavailable,
    VaultNotTradable,
    InvalidVaultCounterparty,
    TxBuildFailed,
    OrderAlreadySubmitted,
    SponsoredTxNotReady,
    InvalidSignedTx,

    InvalidAmount,
    VaultAuthorizationFailed,
    VaultProjectionUnavailable,
    VaultSafetyMode,
    VaultPendingCapital,
    VaultTradingClosed,
    VaultMismatch,
    VaultGovernanceRequired,
    VaultSwapInvalidState,
    VaultSwapInvalidAmount,
    VaultSwapIdempotencyConflict,
    VaultSwapQuoteExpired,
    VaultSwapSignedTxRejected,
    VaultSwapOracleStale,
    VaultSwapRouteUnavailable,
    VaultSwapServiceUnavailable,
}

impl WsErrorCode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

impl std::fmt::Display for WsErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::WsErrorCode;

    #[test]
    fn known_code_maps_to_variant() {
        let code: WsErrorCode = serde_json::from_str("\"trading_paused\"").expect("parse");
        assert_eq!(code, WsErrorCode::TradingPaused);
        assert_eq!(
            serde_json::to_string(&code).expect("ser"),
            "\"trading_paused\""
        );
    }

    #[test]
    fn unsupported_protocol_version_maps_to_typed_variant() {
        let code: WsErrorCode =
            serde_json::from_str("\"unsupported_protocol_version\"").expect("parse");
        assert_eq!(code, WsErrorCode::UnsupportedProtocolVersion);
        assert_eq!(
            serde_json::to_string(&code).expect("ser"),
            "\"unsupported_protocol_version\""
        );
    }

    #[test]
    fn unknown_code_is_rejected() {
        assert!(serde_json::from_str::<WsErrorCode>("\"brand_new_code\"").is_err());
    }
}
