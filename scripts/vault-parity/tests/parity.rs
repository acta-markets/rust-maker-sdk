use acta_contract_sdk::flows::vault_manager::{
    self as c, VaultManagerPlanner, VaultManagerPlannerArgs,
};
use acta_contract_sdk::flows::vault_manager_governance as cg;
use acta_contract_sdk::token::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};
use acta_contract_sdk::{DepositRequest, OracleSource, Vault, WithdrawRequest, pda};
use acta_maker_sdk::VaultDelegatePermissions;
use acta_maker_sdk::chain::vault::{
    self as m, CAPITAL_TX_USERS, VaultAccount, VaultManager, VaultManagerArgs, VaultPhase,
    VaultRequest, VaultRequestKind, VaultSafety,
};
use solana_pubkey::Pubkey;

fn key(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

fn users(count: u8) -> Vec<Pubkey> {
    (0..count).map(|index| key(100 + index)).collect()
}

fn program() -> Pubkey {
    Pubkey::new_from_array(acta_maker_sdk::ACTA_PROGRAM_ID)
}

fn pair(main_token_program: Pubkey) -> (VaultManager, VaultManagerPlanner) {
    let maker = VaultManager::new(VaultManagerArgs {
        program_id: program(),
        vault_id: [7; 32],
        actor: key(1),
        delegate_authority: key(2),
        main_mint: key(3),
        second_mint: key(4),
        share_mint: key(5),
        main_token_program,
        second_token_program: TOKEN_PROGRAM_ID,
    })
    .unwrap();
    let contract = VaultManagerPlanner::new(VaultManagerPlannerArgs {
        program_id: program(),
        vault_id: [7; 32],
        actor: key(1),
        delegate_authority: key(2),
        main_mint: key(3),
        second_mint: key(4),
        share_mint: key(5),
        main_token_program,
        second_token_program: TOKEN_PROGRAM_ID,
    })
    .unwrap();
    (maker, contract)
}

const MAINS: [Pubkey; 2] = [TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID];

#[test]
fn constants_and_program_id() {
    assert_eq!(m::VAULT_CAPITAL_BATCH_USERS, c::VAULT_CAPITAL_BATCH_USERS);
    assert!(CAPITAL_TX_USERS <= c::VAULT_CAPITAL_BATCH_USERS);
    assert_eq!(program(), acta_contract_sdk::ID);
}

#[test]
fn addresses_match() {
    for main in MAINS {
        let (mk, ct) = pair(main);
        let (a, b) = (mk.addresses, ct.addresses);
        assert_eq!(
            [
                a.vault,
                a.policy,
                a.delegate,
                a.global_config,
                a.main_oracle_source,
                a.second_oracle_source,
                a.manager_stake_authority,
                a.vault_main_ata,
                a.vault_second_ata,
                a.manager_stake_ata,
            ],
            [
                b.vault,
                b.policy,
                b.delegate,
                b.global_config,
                b.main_oracle_source,
                b.second_oracle_source,
                b.manager_stake_authority,
                b.vault_main_ata,
                b.vault_second_ata,
                b.manager_stake_ata,
            ],
            "main token program {main}"
        );
    }
}

#[test]
fn pdas_match() {
    let (p, vault, user) = (program(), key(8), key(9));
    let find = |seeds: &[&[u8]]| Pubkey::find_program_address(seeds, &p);
    assert_eq!(
        pda::vault_pda_with_program_id(&p, &[7; 32]),
        find(&[b"vault", &[7; 32]])
    );
    assert_eq!(
        pda::vault_policy_pda_with_program_id(&p, &vault),
        find(&[b"vault_policy", vault.as_ref()])
    );
    assert_eq!(
        pda::vault_delegate_pda_with_program_id(&p, &vault, &user),
        find(&[b"vault_delegate", vault.as_ref(), user.as_ref()])
    );
    assert_eq!(
        pda::manager_stake_pda_with_program_id(&p, &vault),
        find(&[b"manager_stake", vault.as_ref()])
    );
    assert_eq!(
        pda::deposit_request_pda_with_program_id(&p, &vault, &user),
        find(&[b"deposit_request", vault.as_ref(), user.as_ref()])
    );
    assert_eq!(
        pda::deposit_escrow_authority_pda_with_program_id(&p, &vault, &user),
        find(&[b"deposit_escrow", vault.as_ref(), user.as_ref()])
    );
    assert_eq!(
        pda::withdraw_request_pda_with_program_id(&p, &vault, &user),
        find(&[b"withdraw_request", vault.as_ref(), user.as_ref()])
    );
    assert_eq!(
        pda::withdraw_escrow_authority_pda_with_program_id(&p, &vault, &user),
        find(&[b"withdraw_escrow", vault.as_ref(), user.as_ref()])
    );
    assert_eq!(
        pda::pending_vault_update_pda_with_program_id(&p, &vault, 4),
        find(&[b"pending_update", vault.as_ref(), &[4]])
    );
    assert_eq!(
        pda::global_config_pda_with_program_id(&p),
        find(&[b"global_config"])
    );
    assert_eq!(
        pda::oracle_source_pda_with_program_id(&p, &user),
        find(&[b"oracle_source", user.as_ref()])
    );
}

#[test]
fn capital_instructions_match() {
    for main in MAINS {
        let (mk, ct) = pair(main);
        let (a, b) = (key(10), key(11));
        let mut pairs = vec![
            (
                mk.set_accepting_deposits(false),
                ct.set_accepting_deposits(false),
            ),
            (
                mk.set_accepting_deposits(true),
                ct.set_accepting_deposits(true),
            ),
            (mk.process_equity(a, b), ct.process_equity(a, b)),
            (
                mk.process_idle_withdrawal(key(12), a, b),
                ct.process_idle_withdrawal(key(12), a, b),
            ),
            (mk.finalize_cycle(), ct.finalize_cycle()),
            (
                mk.process_final_withdrawal(key(12)),
                ct.process_final_withdrawal(key(12)),
            ),
            (
                mk.process_final_withdrawal_with_share_destination(key(12), Some(key(13))),
                ct.process_final_withdrawal_with_share_destination(key(12), Some(key(13))),
            ),
        ];
        for count in 1..=4 {
            let u = users(count);
            pairs.push((
                mk.process_deposits(&u).unwrap(),
                ct.process_deposits(&u).unwrap(),
            ));
            pairs.push((
                mk.process_withdrawals(&u).unwrap(),
                ct.process_withdrawals(&u).unwrap(),
            ));
        }
        let deposit_dest = [
            (key(20), Some(key(21)), None),
            (key(22), None, Some(key(23))),
        ];
        pairs.push((
            mk.process_deposits_with_destinations(&deposit_dest)
                .unwrap(),
            ct.process_deposits_with_destinations(&deposit_dest)
                .unwrap(),
        ));
        let withdraw_dest = [(key(20), Some(key(21))), (key(22), None)];
        pairs.push((
            mk.process_withdrawals_with_share_destinations(&withdraw_dest)
                .unwrap(),
            ct.process_withdrawals_with_share_destinations(&withdraw_dest)
                .unwrap(),
        ));
        for (index, (maker, contract)) in pairs.iter().enumerate() {
            assert_eq!(maker, contract, "capital #{index}, main {main}");
        }

        let many = users(9);
        let chunks = |f: fn(&VaultManagerPlanner, &[Pubkey]) -> acta_contract_sdk::Instruction| {
            many.chunks(CAPITAL_TX_USERS)
                .map(|batch| f(&ct, batch))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            mk.process_deposit_batches(&many).unwrap(),
            chunks(|ct, batch| ct.process_deposits(batch).unwrap())
        );
        assert_eq!(
            mk.process_withdrawal_batches(&many).unwrap(),
            chunks(|ct, batch| ct.process_withdrawals(batch).unwrap())
        );
    }
}

#[test]
fn batch_validation_matches() {
    for main in MAINS {
        let (mk, ct) = pair(main);
        for u in [vec![], vec![key(1), key(1)], users(4), users(5)] {
            assert_eq!(
                mk.process_deposits(&u).is_ok(),
                ct.process_deposits(&u).is_ok()
            );
            assert_eq!(
                mk.process_withdrawals(&u).is_ok(),
                ct.process_withdrawals(&u).is_ok()
            );
        }
    }
}

#[test]
fn batch_shapes_and_compute_units_match() {
    for main in MAINS {
        let (mk, ct) = pair(main);
        for count in 1..=4 {
            let u = users(count);
            for (maker, contract) in [
                (
                    mk.deposit_batch_shape_conservative(&u),
                    ct.deposit_batch_shape_conservative(&u),
                ),
                (
                    mk.withdrawal_batch_shape_conservative(&u),
                    ct.withdrawal_batch_shape_conservative(&u),
                ),
            ] {
                assert_eq!(
                    (
                        maker.users,
                        maker.created_destinations,
                        maker.derivation_steps
                    ),
                    (
                        contract.users,
                        contract.created_destinations,
                        contract.derivation_steps
                    ),
                    "{count} users, main {main}"
                );
                assert_eq!(
                    m::vault_capital_batch_compute_units(maker),
                    c::vault_capital_batch_compute_units(contract),
                    "{count} users, main {main}"
                );
            }
            assert_eq!(
                m::vault_capital_batch_compute_units(mk.deposit_batch_shape_conservative(&u)),
                ct.deposit_batch_compute_units_conservative(&u)
            );
            assert_eq!(
                m::vault_capital_batch_compute_units(mk.withdrawal_batch_shape_conservative(&u)),
                ct.withdrawal_batch_compute_units_conservative(&u)
            );
        }
        for shape in [(0, 0, 0), (1, 2, 3), (4, 8, 40), (4, 0, 0)] {
            assert_eq!(
                m::vault_capital_batch_compute_units(m::CapitalBatchShape {
                    users: shape.0,
                    created_destinations: shape.1,
                    derivation_steps: shape.2,
                }),
                c::vault_capital_batch_compute_units(c::CapitalBatchShape {
                    users: shape.0,
                    created_destinations: shape.1,
                    derivation_steps: shape.2,
                }),
                "{shape:?}"
            );
        }
    }
}

#[test]
fn governance_instructions_match() {
    use cg::VaultGovernanceUpdateKind as CK;
    use m::VaultGovernanceUpdateKind as MK;
    for main in MAINS {
        let (mk, ct) = pair(main);
        let mp = m::VaultPolicyUpdate {
            max_open_positions: 3,
            max_rebalance_drift_bps: 150,
            min_main_liquidity_bps: 2_500,
        };
        let cp = cg::VaultPolicyUpdate {
            max_open_positions: 3,
            max_rebalance_drift_bps: 150,
            min_main_liquidity_bps: 2_500,
        };
        let updates = [
            (
                m::VaultGovernanceUpdate::CapLimit(u64::MAX - 3),
                cg::VaultGovernanceUpdate::CapLimit(u64::MAX - 3),
            ),
            (
                m::VaultGovernanceUpdate::Fees {
                    performance_bps: 2_000,
                    management_bps: 200,
                },
                cg::VaultGovernanceUpdate::Fees {
                    performance_bps: 2_000,
                    management_bps: 200,
                },
            ),
            (
                m::VaultGovernanceUpdate::HwmReset(u128::MAX - 9),
                cg::VaultGovernanceUpdate::HwmReset(u128::MAX - 9),
            ),
            (
                m::VaultGovernanceUpdate::Capabilities {
                    value: 0b101,
                    mask: 0b111,
                },
                cg::VaultGovernanceUpdate::Capabilities {
                    value: 0b101,
                    mask: 0b111,
                },
            ),
            (
                m::VaultGovernanceUpdate::Policy(mp),
                cg::VaultGovernanceUpdate::Policy(cp),
            ),
            (
                m::VaultGovernanceUpdate::GovernanceAuthority(key(30)),
                cg::VaultGovernanceUpdate::GovernanceAuthority(key(30)),
            ),
            (
                m::VaultGovernanceUpdate::Capabilities {
                    value: 0b1000,
                    mask: 0b1000,
                },
                cg::VaultGovernanceUpdate::Capabilities {
                    value: 0b1000,
                    mask: 0b1000,
                },
            ),
        ];
        for (maker, contract) in updates {
            let (maker, contract) = (mk.propose_update(maker), ct.propose_update(contract));
            assert_eq!(maker.is_ok(), contract.is_ok(), "{maker:?} vs {contract:?}");
            if let (Ok(maker), Ok(contract)) = (maker, contract) {
                assert_eq!(maker, contract);
            }
        }
        let prices = Some((key(10), key(11)));
        for (mkind, ckind) in [
            (MK::CapLimit, CK::CapLimit),
            (MK::Fees, CK::Fees),
            (MK::HwmReset, CK::HwmReset),
            (MK::Capabilities, CK::Capabilities),
            (MK::Policy, CK::Policy),
            (MK::GovernanceAuthority, CK::GovernanceAuthority),
        ] {
            assert_eq!(mkind as u8, ckind as u8);
            assert_eq!(
                mk.apply_update(mkind, key(31), prices).unwrap(),
                ct.apply_update(ckind, key(31), prices).unwrap(),
                "apply {ckind:?}"
            );
            assert_eq!(
                mk.apply_update(mkind, key(31), None).is_ok(),
                ct.apply_update(ckind, key(31), None).is_ok(),
                "apply unpriced {ckind:?}"
            );
            assert_eq!(
                mk.cancel_update(mkind, key(31)),
                ct.cancel_update(ckind, key(31)),
                "cancel {ckind:?}"
            );
        }
        assert_eq!(
            mk.rotate_delegate(key(32), VaultDelegatePermissions::ALL, 1_900_000_000)
                .unwrap(),
            ct.rotate_delegate(key(32), VaultDelegatePermissions::ALL.bits(), 1_900_000_000)
                .unwrap()
        );
        assert_eq!(
            VaultDelegatePermissions::KNOWN_MASK,
            c::VAULT_DELEGATE_PERMISSIONS_KNOWN_MASK
        );
        for (delegate, bits) in [(key(32), 1 << 6), (key(32), 0), (Pubkey::default(), 1)] {
            assert_eq!(
                mk.rotate_delegate(delegate, VaultDelegatePermissions::from_bits(bits), 0)
                    .is_ok(),
                ct.rotate_delegate(delegate, bits, 0).is_ok(),
                "rotate {delegate} {bits}"
            );
        }
        assert_eq!(mk.disable_delegate(), ct.disable_delegate());
    }
}

fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index * 31 + 17) as u8).collect()
}

#[test]
fn vault_account_decodes_like_contract() {
    for (main_kind, second_kind) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        let mut data = patterned(VaultAccount::LEN);
        data[..5].copy_from_slice(&[9, 1, 255, 3, 1]);
        data[12..16].copy_from_slice(&3u32.to_le_bytes());
        data[402] = main_kind;
        data[403] = second_kind;
        assert_eq!(VaultAccount::LEN, Vault::LEN);
        let mv = VaultAccount::parse(&data).unwrap();
        let cv = Vault::from_bytes(&data).unwrap();
        let kind = |program: Pubkey| u8::from(program == TOKEN_2022_PROGRAM_ID);
        let phase = match mv.phase {
            VaultPhase::Idle => 0,
            VaultPhase::Active => 1,
            VaultPhase::Settling => 2,
            VaultPhase::ProcessingCapital => 3,
            _ => u8::MAX,
        };
        let safety = match mv.safety {
            VaultSafety::Normal => 0,
            VaultSafety::Frozen => 1,
            VaultSafety::EmergencyUnwind => 2,
            VaultSafety::WithdrawOnly => 3,
            VaultSafety::Closing => 4,
            _ => u8::MAX,
        };
        let flags = u32::from(mv.equity_computed) | (u32::from(mv.accepting_deposits) << 1);
        assert_eq!(
            (mv.vault_id, phase, safety, flags,),
            (cv.vault_id, cv.status, cv.safety_mode, cv.accounting_flags)
        );
        assert_eq!(
            (
                mv.governance_authority,
                mv.primary_delegate,
                mv.main_mint,
                mv.second_mint,
                mv.share_mint,
                kind(mv.main_token_program),
                kind(mv.second_token_program),
                mv.total_shares,
                mv.pending_deposit_count,
                mv.pending_withdraw_count,
            ),
            (
                cv.governance_authority,
                cv.primary_delegate,
                cv.main_mint,
                cv.second_mint,
                cv.share_mint,
                cv.main_token_program_kind,
                cv.second_token_program_kind,
                cv.total_shares,
                cv.pending_deposit_count,
                cv.pending_withdraw_count,
            )
        );
    }
}

#[test]
fn requests_and_oracle_source_decode_like_contract() {
    for kind in [VaultRequestKind::Deposit, VaultRequestKind::Withdraw] {
        let mut data = patterned(VaultRequest::LEN);
        data[..2].copy_from_slice(&[kind.discriminator(), 1]);
        let r = VaultRequest::parse(&data, kind).unwrap();
        let contract = match kind {
            VaultRequestKind::Deposit => {
                assert_eq!(VaultRequest::LEN, DepositRequest::LEN);
                let d = DepositRequest::from_bytes(&data).unwrap();
                (d.user, d.vault, d.amount, d.created_at)
            }
            VaultRequestKind::Withdraw => {
                assert_eq!(VaultRequest::LEN, WithdrawRequest::LEN);
                let w = WithdrawRequest::from_bytes(&data).unwrap();
                (w.user, w.vault, w.shares, w.created_at)
            }
        };
        assert_eq!((r.user, r.vault, r.amount, r.created_at), contract);
    }
    let mut data = patterned(OracleSource::LEN);
    data[..2].copy_from_slice(&[acta_contract_sdk::discriminators::ORACLE_SOURCE, 1]);
    let source = OracleSource::from_bytes(&data).unwrap();
    assert_eq!(
        m::oracle_source_pyth_feed(&data).unwrap().as_slice(),
        &source.feeds[32..64]
    );
}
