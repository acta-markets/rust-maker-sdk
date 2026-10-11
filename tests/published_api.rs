use acta_maker_sdk::types::{Decimals, Quantity};
use acta_maker_sdk::ws::types::{PositionSizeRule, TokenInfo, WsChannel, WsEndpointKind};

#[test]
fn token_info_is_constructible_as_in_0_4_3() {
    let token = TokenInfo {
        mint: "So11111111111111111111111111111111111111112".into(),
        decimals: Decimals::new(9),
        size_rule: PositionSizeRule {
            min_size: Quantity::new(1),
            max_size: Quantity::new(2),
            step: Quantity::new(1),
        },
        symbol: "SOL".into(),
    };
    assert_eq!(token.symbol, "SOL");
}

#[test]
fn unknown_channel_and_endpoint_keep_their_0_4_3_discriminants() {
    assert_eq!(WsChannel::Unknown as isize, 6);
    assert_eq!(WsEndpointKind::Unknown as isize, 3);
}

#[cfg(feature = "chain")]
#[test]
fn chain_error_keeps_its_0_4_3_variants() {
    use acta_maker_sdk::chain::ChainError;

    fn published(error: &ChainError) -> &'static str {
        match error {
            #[cfg(feature = "chain-rpc")]
            ChainError::Rpc(_) => "rpc",
            ChainError::InvalidAccountData => "invalid",
            ChainError::AccountOwnerMismatch { .. } => "owner",
            ChainError::AccountDiscriminatorMismatch { .. } => "discriminator",
            ChainError::UninitializedAccount => "uninitialized",
            ChainError::UnknownPositionStatus(_) => "status",
            ChainError::InvalidPositionType(_) => "type",
            ChainError::SettlementAmountOverflow => "overflow",
            ChainError::MissingFundingAccount(_) => "funding",
            ChainError::InvalidNativeTokenProgram(_) => "native program",
            ChainError::InsufficientNativeSol { .. } => "native sol",
            ChainError::NativeSolBudgetOverflow => "budget",
            ChainError::Ix(_) => "ix",
            #[cfg(feature = "chain-rpc")]
            ChainError::Signing(_) => "signing",
        }
    }
    assert_eq!(published(&ChainError::InvalidAccountData), "invalid");
}
