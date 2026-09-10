use super::*;
use solana_sdk::{
    hash::Hash, instruction::AccountMeta, signature::Keypair, transaction::Transaction,
};

#[test]
fn transaction_signing_returns_error_for_missing_signer() {
    let fee_payer = Keypair::new();
    let missing_signer = Pubkey::new_unique();
    let instruction = Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![AccountMeta::new_readonly(missing_signer, true)],
        data: Vec::new(),
    };
    let mut transaction = Transaction::new_with_payer(&[instruction], Some(&fee_payer.pubkey()));
    let signers: [&(dyn Signer + Sync); 1] = [&fee_payer];

    assert!(matches!(
        sign_transaction(&mut transaction, &signers, Hash::new_unique()),
        Err(ChainError::Signing(_))
    ));
}

#[test]
fn transaction_signing_verifies_transaction() {
    let fee_payer = Keypair::new();
    let mut transaction = Transaction::new_with_payer(&[], Some(&fee_payer.pubkey()));
    let signers: [&(dyn Signer + Sync); 1] = [&fee_payer];

    sign_transaction(&mut transaction, &signers, Hash::new_unique()).unwrap();

    assert!(transaction.verify().is_ok());
}
