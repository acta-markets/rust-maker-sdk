#![cfg(feature = "chain")]

use acta_maker_sdk::chain::ix::{
    ChainIxError, FundPositionIxArgs, MakerDepositIxArgs, MakerWithdrawIxArgs,
    UpdateQuoteSigningIxArgs, build_fund_position_ixs, build_maker_deposit_ixs,
    build_maker_withdraw_ixs, build_update_quote_signing_ixs, maker_pda_with_program_id,
};
use solana_sdk::pubkey::Pubkey;

fn test_pubkey(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

fn token_program() -> Pubkey {
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
fn maker_deposit_ixs_structure() {
    let program_id = test_pubkey(1);
    let args = MakerDepositIxArgs {
        maker_owner: test_pubkey(2),
        amount: 1_000_000,
        premium_mint: test_pubkey(3),
        token_program: token_program(),
        create_atas: false,
    };

    let ixs = build_maker_deposit_ixs(&program_id, &args, test_pubkey(10)).unwrap();
    assert_eq!(ixs.len(), 1, "no ATA ixs when create_atas=false");

    let ix = &ixs[0];
    assert_eq!(ix.program_id, program_id);
    assert_eq!(ix.accounts.len(), 5);
    assert_eq!(ix.data[0], 4, "MakerDeposit discriminant");

    let amount_bytes = &ix.data[1..9];
    assert_eq!(
        u64::from_le_bytes(amount_bytes.try_into().unwrap()),
        1_000_000
    );
}

#[test]
fn update_quote_signing_ixs_structure() {
    let program_id = test_pubkey(1);
    let owner = test_pubkey(2);
    let new_key = test_pubkey(3);
    let ixs = build_update_quote_signing_ixs(
        &program_id,
        &UpdateQuoteSigningIxArgs {
            maker_owner: owner,
            new_quote_signing: new_key,
        },
    );
    assert_eq!(ixs.len(), 1);
    let ix = &ixs[0];
    assert_eq!(ix.program_id, program_id);
    assert_eq!(ix.data[0], 3, "UpdateMakerQuoteSigning discriminant");
    assert_eq!(&ix.data[1..], new_key.as_ref());
    assert_eq!(ix.accounts.len(), 2);
    assert_eq!(ix.accounts[0].pubkey, owner);
    assert!(ix.accounts[0].is_signer && !ix.accounts[0].is_writable);
    assert_eq!(
        ix.accounts[1].pubkey,
        maker_pda_with_program_id(&program_id, &owner).0
    );
    assert!(!ix.accounts[1].is_signer && ix.accounts[1].is_writable);
}

#[test]
fn maker_withdraw_ixs_structure() {
    let program_id = test_pubkey(1);
    let args = MakerWithdrawIxArgs {
        maker_owner: test_pubkey(2),
        amount: 500_000,
        premium_mint: test_pubkey(3),
        token_program: token_program(),
        create_atas: false,
    };

    let ixs = build_maker_withdraw_ixs(&program_id, &args, test_pubkey(10)).unwrap();
    assert_eq!(ixs.len(), 1);

    let ix = &ixs[0];
    assert_eq!(ix.accounts.len(), 5);
    assert_eq!(ix.data[0], 5, "MakerWithdraw discriminant");
}

#[test]
fn fund_position_invalid_type() {
    let program_id = test_pubkey(1);
    let args = FundPositionIxArgs {
        holder: test_pubkey(2),
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
        holder: test_pubkey(2),
        position_pda: test_pubkey(3),
        market_pda: test_pubkey(4),
        position_type: 0,
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

fn digest(ixs: &[solana_sdk::instruction::Instruction]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for ix in ixs {
        hasher.update(ix.program_id.as_ref());
        for meta in &ix.accounts {
            hasher.update(meta.pubkey.as_ref());
            hasher.update([u8::from(meta.is_signer), u8::from(meta.is_writable)]);
        }
        hasher.update(&ix.data);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Captured before the maker custody rename: the builders must emit the same transactions.
#[test]
fn custody_deposit_and_withdraw_keep_their_pre_rename_bytes() {
    let deposit = build_maker_deposit_ixs(
        &test_pubkey(1),
        &MakerDepositIxArgs {
            maker_owner: test_pubkey(2),
            amount: 77,
            premium_mint: test_pubkey(3),
            token_program: token_program(),
            create_atas: true,
        },
        test_pubkey(4),
    )
    .unwrap();
    assert_eq!(
        digest(&deposit),
        "dada979079829681321cb31fb2e2a43ff2cb2ce2d0a49ff66aa1b33abaab22b8"
    );
    let withdraw = build_maker_withdraw_ixs(
        &test_pubkey(1),
        &MakerWithdrawIxArgs {
            maker_owner: test_pubkey(2),
            amount: 88,
            premium_mint: test_pubkey(3),
            token_program: token_program(),
            create_atas: true,
        },
        test_pubkey(4),
    )
    .unwrap();
    assert_eq!(
        digest(&withdraw),
        "b18e645cc78df6690272d7ab405bd11431dbed3f36dc0f683623f98435f38c60"
    );
}

/// Captured before the writer/holder rename: the same transaction and event bytes.
#[test]
fn position_role_builders_and_events_keep_their_pre_rename_bytes() {
    let fund = acta_maker_sdk::chain::build_fund_position_ixs(
        &test_pubkey(1),
        &acta_maker_sdk::chain::FundPositionIxArgs {
            holder: test_pubkey(2),
            position_pda: test_pubkey(3),
            market_pda: test_pubkey(4),
            position_type: 0,
            underlying_mint: test_pubkey(5),
            quote_mint: test_pubkey(6),
            underlying_token_program_id: token_program(),
            quote_token_program_id: token_program(),
            create_atas: true,
        },
        test_pubkey(7),
    )
    .unwrap();
    assert_eq!(
        digest(&fund),
        "2f8549c38dd312c6feba9e0c02f8256cecdce1fcfaf9be107ada6829aef4aeef"
    );

    let mut settle = vec![65, 67, 84, 65, 2, 79];
    for byte in [5u8, 6, 7, 8, 9] {
        settle.extend_from_slice(&[byte; 32]);
    }
    settle.extend_from_slice(&[4, 0]);
    settle.extend_from_slice(&7u64.to_le_bytes());
    settle.extend_from_slice(&8u64.to_le_bytes());
    let Some(acta_maker_sdk::chain::events::PositionEvent::SpreadSettled(event)) =
        acta_maker_sdk::chain::events::decode_position_event(&settle).unwrap()
    else {
        panic!("not a SettleSpread");
    };
    assert_eq!(
        (
            event.writer,
            event.holder,
            event.holder_payoff,
            event.writer_return
        ),
        ([6; 32], [7; 32], 7, 8)
    );
}
