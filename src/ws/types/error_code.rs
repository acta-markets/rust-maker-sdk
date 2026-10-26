use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{EnumString, IntoStaticStr};

/// `Generic.code` / `AuthError.reason`. Codes newer than this SDK land in `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
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
    InvalidPubkey,
    PubkeyMismatch,
    InvalidSignature,
    InvalidOperator,
    OperatorMismatch,
    InvalidVault,
    VaultAuthNotStarted,
    VaultAuthRequired,
    VaultResumeNotSupported,
    VaultOperatorUnauthorized,
    VaultAuthorizationUnavailable,
    VaultSessionInvalidated,

    InvalidVaultSubscription,
    SubscriptionScopeTooLarge,
    HistoryProjectionIncomplete,
    InvalidPositionStatus,
    InvalidCursor,
    InvalidQueryScope,
    MarketNotFound,
    InvalidProgramId,

    InvalidStrikeNotional,
    InsufficientCollateral,
    InvalidQuantitySizeRule,
    MissingSizeRuleForUnderlyingMint,
    InvalidUnderlyingMintKey,
    MissingTickForUnderlyingMint,
    LadderStrikeSourceUnavailable,
    InvalidStrikes,
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

    #[strum(default)]
    Unknown(String),
}

impl WsErrorCode {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(code) => code,
            known => known.into(),
        }
    }
}

impl std::fmt::Display for WsErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for WsErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WsErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        match code.parse() {
            Ok(known) => Ok(known),
            Err(_) => Ok(Self::Unknown(code)),
        }
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
    fn unknown_code_round_trips() {
        let code: WsErrorCode = serde_json::from_str("\"brand_new_code\"").expect("parse");
        assert_eq!(code, WsErrorCode::Unknown("brand_new_code".into()));
        assert_eq!(code.as_str(), "brand_new_code");
        assert_eq!(
            serde_json::to_string(&code).expect("ser"),
            "\"brand_new_code\""
        );
    }
}
