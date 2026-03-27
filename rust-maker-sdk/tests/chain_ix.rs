#![cfg(feature = "chain")]

use acta_maker_sdk::chain::ix::{
    ChainIxError, DepositPremiumIxArgs, FundPositionIxArgs, WithdrawPremiumIxArgs,
    build_deposit_premium_ixs, build_fund_position_ixs, build_withdraw_premium_ixs,
};
use solana_sdk::pubkey::Pubkey;

fn test_pubkey(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

fn token_program() -> Pubkey {
    // SPL Token program ID
    Pubkey::new_from_array({
        let mut arr = [0u8; 32];
        arr[0] = 6;
        arr[1] = 221;
        arr[2] = 246;
        arr[3] = 225;
        arr[4] = 215;
        arr[5] = 101;
        arr[6] = 161;
        arr
    })
}

#[test]
fn deposit_premium_ixs_structure() {
    let program_id = test_pubkey(1);
    let args = DepositPremiumIxArgs {
        maker_owner: test_pubkey(2),
        amount: 1_000_000,
        premium_mint: test_pubkey(3),
        token_program: token_program(),
        create_atas: false,
    };

    let ixs = build_deposit_premium_ixs(&program_id, &args, test_pubkey(10)).unwrap();
    assert_eq!(ixs.len(), 1, "no ATA ixs when create_atas=false");

    let ix = &ixs[0];
    assert_eq!(ix.program_id, program_id);
    assert_eq!(ix.accounts.len(), 5);
    assert_eq!(ix.data[0], 4, "DepositPremium discriminant");

    let amount_bytes = &ix.data[1..9];
    assert_eq!(
        u64::from_le_bytes(amount_bytes.try_into().unwrap()),
        1_000_000
    );
}

#[test]
fn withdraw_premium_ixs_structure() {
    let program_id = test_pubkey(1);
    let args = WithdrawPremiumIxArgs {
        maker_owner: test_pubkey(2),
        amount: 500_000,
        premium_mint: test_pubkey(3),
        token_program: token_program(),
        create_atas: false,
    };

    let ixs = build_withdraw_premium_ixs(&program_id, &args, test_pubkey(10)).unwrap();
    assert_eq!(ixs.len(), 1);

    let ix = &ixs[0];
    assert_eq!(ix.accounts.len(), 5);
    assert_eq!(ix.data[0], 5, "WithdrawPremium discriminant");
}

#[test]
fn fund_position_invalid_type() {
    let program_id = test_pubkey(1);
    let args = FundPositionIxArgs {
        maker_owner: test_pubkey(2),
        position_pda: test_pubkey(3),
        market_pda: test_pubkey(4),
        position_type: 99,
        underlying_mint: test_pubkey(5),
        quote_mint: test_pubkey(6),
        underlying_token_program_id: token_program(),
        quote_token_program_id: token_program(),
        create_atas: false,
    };

    let err = build_fund_position_ixs(&program_id, &args, test_pubkey(10)).unwrap_err();
    assert!(matches!(err, ChainIxError::InvalidPositionType(99)));
}

#[test]
fn fund_position_covered_call_uses_quote_mint() {
    let program_id = test_pubkey(1);
    let quote_mint = test_pubkey(6);
    let underlying_mint = test_pubkey(5);
    let args = FundPositionIxArgs {
        maker_owner: test_pubkey(2),
        position_pda: test_pubkey(3),
        market_pda: test_pubkey(4),
        position_type: 0, // covered call → uses quote_mint
        underlying_mint,
        quote_mint,
        underlying_token_program_id: token_program(),
        quote_token_program_id: token_program(),
        create_atas: false,
    };

    let ixs = build_fund_position_ixs(&program_id, &args, test_pubkey(10)).unwrap();
    assert_eq!(ixs.len(), 1);

    let ix = &ixs[0];
    assert_eq!(ix.data[0], 9, "DepositFundsToPosition discriminant");
    assert_eq!(ix.accounts.len(), 7);
}
