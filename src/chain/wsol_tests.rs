use super::*;

fn token_account(owner: Pubkey, amount: u64) -> Account {
    let mut data = vec![0; TOKEN_ACCOUNT_BASE_LEN];
    data[TOKEN_ACCOUNT_MINT_OFFSET..TOKEN_ACCOUNT_MINT_OFFSET + 32]
        .copy_from_slice(NATIVE_MINT.as_ref());
    data[TOKEN_ACCOUNT_OWNER_OFFSET..TOKEN_ACCOUNT_OWNER_OFFSET + 32]
        .copy_from_slice(owner.as_ref());
    data[TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_AMOUNT_OFFSET + 8]
        .copy_from_slice(&amount.to_le_bytes());
    Account {
        lamports: 0,
        data,
        owner: TOKEN_PROGRAM_ID,
        executable: false,
        rent_epoch: 0,
    }
}

#[test]
fn wrap_decision_accounts_for_existing_wsol_and_reserve() {
    assert_eq!(
        decide_wrap(
            700,
            1_000,
            DEFAULT_SOL_RESERVE_LAMPORTS + 300,
            DEFAULT_SOL_RESERVE_LAMPORTS,
            0,
        ),
        WrapDecision::Wrap { wrap_lamports: 300 }
    );
}

#[test]
fn wrap_decision_rejects_balance_below_reserve_boundary() {
    let needed = DEFAULT_SOL_RESERVE_LAMPORTS + 1_000;
    assert_eq!(
        decide_wrap(0, 1_000, needed - 1, DEFAULT_SOL_RESERVE_LAMPORTS, 0),
        WrapDecision::InsufficientNativeSol {
            needed_lamports: needed,
            available_lamports: needed - 1,
        }
    );
    assert_eq!(
        decide_wrap(0, 1_000, needed, DEFAULT_SOL_RESERVE_LAMPORTS, 0),
        WrapDecision::Wrap {
            wrap_lamports: 1_000
        }
    );
}

#[test]
fn native_budget_includes_wrap_rent_fee_and_reserve() {
    let budget = NativeSolBudget {
        owner: Pubkey::new_unique(),
        wrap_lamports: 1_000,
        ata_rent_lamports: 2_039_280,
        reserve_lamports: 1_200_000,
    };
    let needed = 1_000 + 2_039_280 + 5_000 + 1_200_000;

    assert!(budget.validate(needed, 5_000).is_ok());
    assert!(matches!(
        budget.validate(needed - 1, 5_000),
        Err(ChainError::InsufficientNativeSol {
            needed_lamports,
            available_lamports,
        }) if needed_lamports == needed && available_lamports == needed - 1
    ));
}

#[test]
fn native_budget_overflow_fails_closed() {
    let budget = NativeSolBudget {
        owner: Pubkey::new_unique(),
        wrap_lamports: u64::MAX,
        ata_rent_lamports: 1,
        reserve_lamports: 0,
    };

    assert!(matches!(
        budget.validate(u64::MAX, 0),
        Err(ChainError::NativeSolBudgetOverflow)
    ));
}

#[test]
fn existing_wsol_account_is_validated_before_its_balance_is_used() {
    let owner = Pubkey::new_unique();
    assert_eq!(
        parse_wsol_balance(&token_account(owner, 42), &owner, &TOKEN_PROGRAM_ID).unwrap(),
        42
    );

    let other_owner = Pubkey::new_unique();
    assert!(matches!(
        parse_wsol_balance(&token_account(owner, 42), &other_owner, &TOKEN_PROGRAM_ID),
        Err(ChainError::InvalidAccountData)
    ));
}

#[test]
fn wrap_instruction_order_is_transfer_then_sync_native() {
    let owner = Pubkey::new_unique();
    let ata = Pubkey::new_unique();
    let transfer = build_system_transfer_ix(&owner, &ata, 9);
    let sync = build_sync_native_ix(&TOKEN_PROGRAM_ID, &ata);
    assert_eq!(transfer.program_id, SYSTEM_PROGRAM_ID);
    assert_eq!(
        &transfer.data[..4],
        &SYSTEM_INSTRUCTION_TRANSFER.to_le_bytes()
    );
    assert_eq!(sync.data, vec![SPL_TOKEN_SYNC_NATIVE]);
}
