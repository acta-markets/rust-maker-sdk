use super::*;
use solana_sdk::{hash::Hash, instruction::AccountMeta, signature::Keypair};

#[test]
fn transaction_builder_returns_error_for_missing_signer() {
    let fee_payer = Keypair::new();
    let missing_signer = Pubkey::new_unique();
    let instruction = Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![AccountMeta::new_readonly(missing_signer, true)],
        data: Vec::new(),
    };
    let signers: [&dyn Signer; 1] = [&fee_payer];

    assert!(matches!(
        build_signed_transaction(
            vec![instruction],
            fee_payer.pubkey(),
            &signers,
            Hash::new_unique(),
        ),
        Err(ChainError::Signing(_))
    ));
}
