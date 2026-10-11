use acta_maker_sdk::ws::types::{WsChannel, WsEndpointKind};

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
