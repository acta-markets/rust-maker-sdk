//! Golden bytes recorded from the contract's `acta-contract-sdk`.

use sha2::{Digest, Sha256};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

use super::{
    VaultAccount, VaultChainError, VaultGovernanceUpdate, VaultGovernanceUpdateKind, VaultManager,
    VaultManagerArgs, VaultPhase, VaultPolicyUpdate, VaultPositionRef, VaultRequest,
    VaultRequestKind, VaultSafety, oracle_source_pyth_feed, pda, vault_capital_batch_compute_units,
};
use crate::VaultDelegatePermissions;
use crate::chain::ix::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};

fn key(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

fn users(count: u8) -> Vec<Pubkey> {
    (0..count).map(|index| key(100 + index)).collect()
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes)[..8])
}

/// `data hex / account count / digest of program id and metas`.
fn ix(ix: &Instruction) -> String {
    let mut bytes = ix.program_id.to_bytes().to_vec();
    for meta in &ix.accounts {
        bytes.extend(meta.pubkey.to_bytes());
        bytes.extend([u8::from(meta.is_signer), u8::from(meta.is_writable)]);
    }
    format!(
        "{}/{}/{}",
        hex::encode(&ix.data),
        ix.accounts.len(),
        digest(&bytes)
    )
}

fn manager(main_token_program: Pubkey) -> VaultManager {
    VaultManager::new(VaultManagerArgs {
        program_id: Pubkey::new_from_array(crate::ACTA_PROGRAM_ID),
        vault_id: [7; 32],
        actor: key(1),
        delegate_authority: key(2),
        main_mint: key(3),
        second_mint: key(4),
        share_mint: key(5),
        main_token_program,
        second_token_program: TOKEN_PROGRAM_ID,
    })
    .unwrap()
}

struct Golden {
    token_program: Pubkey,
    addresses: &'static str,
    capital: [&'static str; 15],
    shapes: &'static str,
    /// Conservative compute units for 1..=3 users: (deposit, withdrawal).
    all_missing: [(u32, u32); 3],
}

const GOLDEN: [Golden; 2] = [
    Golden {
        token_program: TOKEN_PROGRAM_ID,
        addresses: "b7803a2b97b482d1",
        capital: [
            "2e00/5/f779cd53a5637a79",
            "2e01/5/f779cd53a5637a79",
            "25/17/0cce5b8f23f0b8b4",
            "25/18/8bc58b79ee932ac8",
            "28/7/f0c7f42cc0a16cbd",
            "40/21/f93f7bc7d918f7ec",
            "40/21/8420fd118883d1c0",
            "27/18/76f93b7e4e221903",
            "26/17/19e49dfe00549d7e",
            "27/25/50fdb4c8ec7ee764",
            "26/24/3c2076dbc29893a0",
            "27/32/82c41b8b12030e34",
            "26/31/3ac9526f9287dc5f",
            "27/25/cc1bf83688a67582",
            "26/24/5443b914191d8422",
        ],
        shapes: "494335a5087e54be",
        all_missing: [(130_000, 130_000), (216_400, 209_200), (286_600, 281_200)],
    },
    Golden {
        token_program: TOKEN_2022_PROGRAM_ID,
        addresses: "6c68d1785a8a76a2",
        capital: [
            "2e00/5/f779cd53a5637a79",
            "2e01/5/f779cd53a5637a79",
            "25/17/335b18a1c174d7d7",
            "25/18/f53b718cd1792e02",
            "28/7/7a32261d5c71ada7",
            "40/21/9b04ca82a25313bf",
            "40/21/2c5400de9cff0c92",
            "27/18/a790fb648e3c9a58",
            "26/17/fdbf70b09ecc621d",
            "27/25/bff1bd589080ad28",
            "26/24/a95faa21b1accbe2",
            "27/32/70752bc4a123fe16",
            "26/31/8ba5ea72877a303a",
            "27/25/9618893ffdb13bce",
            "26/24/276b0674cc3714f7",
        ],
        shapes: "b58c0c14591fefc9",
        all_missing: [(130_000, 126_400), (200_200, 191_200), (268_600, 263_200)],
    },
];

#[test]
fn addresses_and_capital_instructions_match_golden() {
    for golden in GOLDEN {
        let m = manager(golden.token_program);
        let a = m.addresses;
        let addresses: Vec<u8> = [
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
        ]
        .iter()
        .flat_map(Pubkey::to_bytes)
        .collect();
        assert_eq!(digest(&addresses), golden.addresses);

        let (main, second) = (key(10), key(11));
        let mut capital = vec![
            m.set_accepting_deposits(false),
            m.set_accepting_deposits(true),
            m.process_equity(main, second),
            m.process_idle_withdrawal(key(12), main, second),
            m.finalize_cycle(),
            m.process_final_withdrawal(key(12)),
            m.process_final_withdrawal_with_share_destination(key(12), Some(key(13))),
        ];
        for count in 1..=3 {
            capital.push(m.process_deposits(&users(count)).unwrap());
            capital.push(m.process_withdrawals(&users(count)).unwrap());
        }
        capital.push(
            m.process_deposits_with_destinations(&[
                (key(20), Some(key(21)), None),
                (key(22), None, Some(key(23))),
            ])
            .unwrap(),
        );
        capital.push(
            m.process_withdrawals_with_share_destinations(&[
                (key(20), Some(key(21))),
                (key(22), None),
            ])
            .unwrap(),
        );
        assert_eq!(capital.iter().map(ix).collect::<Vec<_>>(), golden.capital);
    }
}

#[test]
fn batch_validation_rejects_empty_duplicate_and_oversized() {
    let m = manager(TOKEN_PROGRAM_ID);
    for users in [vec![], vec![key(1), key(1)], users(4)] {
        assert!(m.process_deposits(&users).is_err());
        assert!(m.process_withdrawals(&users).is_err());
    }
}

#[test]
fn batch_shapes_and_compute_units_match_golden() {
    for golden in GOLDEN {
        let m = manager(golden.token_program);
        let mut shapes = Vec::new();
        for count in 0..=5 {
            let users = users(count);
            for missing in 0..=users.len() {
                let rest = users.len() - missing;
                for shape in [
                    m.deposit_batch_shape_with_destinations(&users, missing, rest),
                    m.withdrawal_batch_shape_with_destinations(&users, missing, rest),
                ] {
                    for value in [
                        shape.users,
                        shape.created_destinations,
                        shape.derivation_steps,
                        vault_capital_batch_compute_units(shape),
                    ] {
                        shapes.extend(value.to_le_bytes());
                    }
                }
            }
        }
        assert_eq!(digest(&shapes), golden.shapes);
        let all_missing: Vec<_> = (1..=3)
            .map(|count| {
                let users = users(count);
                (
                    vault_capital_batch_compute_units(m.deposit_batch_shape_with_destinations(
                        &users,
                        users.len(),
                        users.len(),
                    )),
                    vault_capital_batch_compute_units(m.withdrawal_batch_shape_with_destinations(
                        &users,
                        users.len(),
                        users.len(),
                    )),
                )
            })
            .collect();
        assert_eq!(all_missing, golden.all_missing);
    }
}

#[test]
fn pdas_match_golden() {
    let (program, vault, user) = (
        Pubkey::new_from_array(crate::ACTA_PROGRAM_ID),
        key(8),
        key(9),
    );
    let pdas: Vec<u8> = [
        pda::vault(&program, &[7; 32]),
        pda::vault_policy(&program, &vault),
        pda::vault_delegate(&program, &vault, &user),
        pda::manager_stake(&program, &vault),
        pda::deposit_request(&program, &vault, &user),
        pda::deposit_escrow_authority(&program, &vault, &user),
        pda::withdraw_request(&program, &vault, &user),
        pda::withdraw_escrow_authority(&program, &vault, &user),
        pda::pending_vault_update(&program, &vault, 4),
        pda::global_config(&program),
        pda::oracle_source(&program, &user),
    ]
    .iter()
    .flat_map(|(address, bump)| address.to_bytes().into_iter().chain([*bump]))
    .collect();
    assert_eq!(digest(&pdas), "2a461baad1349d70");
}

#[test]
fn fund_maker_position_matches_golden() {
    let golden = [
        (
            TOKEN_PROGRAM_ID,
            [
                "2b/9/968f0e546b17c484",
                "2b/9/35f9407a4dc98214",
                "2b/9/6524aed42e403599",
                "2b/9/41bd28931f9f9789",
            ],
        ),
        (
            TOKEN_2022_PROGRAM_ID,
            [
                "2b/9/968f0e546b17c484",
                "2b/9/42dcbbc2459ceb28",
                "2b/9/0edf33941806db11",
                "2b/9/41bd28931f9f9789",
            ],
        ),
    ];
    for (token_program, expected) in golden {
        let m = manager(token_program);
        let mut built = Vec::new();
        for (underlying_mint, quote_mint) in [(key(3), key(4)), (key(4), key(3))] {
            for is_put in [false, true] {
                built.push(ix(&m
                    .fund_maker_position(&VaultPositionRef {
                        underlying_mint,
                        quote_mint,
                        expiry_ts: 1_900_000_000,
                        is_put,
                        order_id: [9; 32],
                    })
                    .unwrap()));
            }
        }
        assert_eq!(built, expected);
    }
    let held = VaultPositionRef {
        underlying_mint: key(3),
        quote_mint: key(6),
        expiry_ts: 1_900_000_000,
        is_put: false,
        order_id: [9; 32],
    };
    let m = manager(TOKEN_PROGRAM_ID);
    assert!(m.fund_maker_position(&held).is_err());
    for held in [
        VaultPositionRef {
            quote_mint: key(4),
            expiry_ts: 0,
            ..held
        },
        VaultPositionRef {
            quote_mint: key(4),
            order_id: [0; 32],
            ..held
        },
    ] {
        assert!(m.fund_maker_position(&held).is_err());
    }
}

#[test]
fn governance_instructions_match_golden() {
    use VaultGovernanceUpdateKind as Kind;
    let m = manager(TOKEN_2022_PROGRAM_ID);
    let policy = VaultPolicyUpdate {
        max_open_positions: 3,
        max_rebalance_drift_bps: 150,
        min_main_liquidity_bps: 2_500,
    };
    let mut actual: Vec<_> = [
        VaultGovernanceUpdate::CapLimit(u64::MAX - 3),
        VaultGovernanceUpdate::Fees {
            performance_bps: 2_000,
            management_bps: 200,
        },
        VaultGovernanceUpdate::HwmReset(u128::MAX - 9),
        VaultGovernanceUpdate::Capabilities {
            value: 0b101,
            mask: 0b111,
        },
        VaultGovernanceUpdate::Policy(policy),
        VaultGovernanceUpdate::GovernanceAuthority(key(30)),
    ]
    .into_iter()
    .map(|update| ix(&m.propose_update(update).unwrap()))
    .collect();
    let prices = Some((key(10), key(11)));
    let mut unpriced = Vec::new();
    for kind in [
        Kind::CapLimit,
        Kind::Fees,
        Kind::HwmReset,
        Kind::Capabilities,
        Kind::Policy,
        Kind::GovernanceAuthority,
    ] {
        actual.push(ix(&m.apply_update(kind, key(31), prices).unwrap()));
        actual.push(ix(&m.cancel_update(kind, key(31))));
        unpriced.push(m.apply_update(kind, key(31), None).is_ok());
    }
    actual.push(ix(&m
        .rotate_delegate(key(32), VaultDelegatePermissions::ALL, 1_900_000_000)
        .unwrap()));
    actual.push(ix(&m.disable_delegate()));
    assert_eq!(
        actual,
        [
            "3900fcffffffffffffff000000000000000000000000000000000000000000000000/5/5ec5dd1abd480928",
            "3901d007c80000000000000000000000000000000000000000000000000000000000/5/45c03854ef6a24bc",
            "3902f6ffffffffffffffffffffffffffffff00000000000000000000000000000000/5/87e39b3943882b86",
            "39030500000007000000000000000000000000000000000000000000000000000000/5/8e0cd501f9328fa8",
            "390403009600c4090000000000000000000000000000000000000000000000000000/5/48194846a6c25808",
            "39051e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e/5/9104fa0231f4964e",
            "3a00/4/d0371903118ce95d",
            "3b00/5/abdc1cc51536dfb6",
            "3a01/18/a48abe4390632431",
            "3b01/5/60182776ff64aff8",
            "3a02/18/6be772550a5b0b31",
            "3b02/5/5441df267873fd5b",
            "3a03/4/dc0663ae22446adb",
            "3b03/5/e77b863dc778acb2",
            "3a04/4/142623051969a573",
            "3b04/5/00a1e87c02e4835b",
            "3a05/4/da408c2b2eb99f79",
            "3b05/5/dd663461db40b769",
            "3120202020202020202020202020202020202020202020202020202020202020203f00000000b33f7100000000/5/db66018087f022ff",
            "48/3/4a35ed45ecc4ab38",
        ]
    );
    assert_eq!(unpriced, [true, false, false, true, true, true]);
    assert!(
        m.rotate_delegate(key(32), VaultDelegatePermissions::from_bits(1 << 6), 0)
            .is_err()
    );
}

fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index * 31 + 17) as u8).collect()
}

#[test]
fn vault_account_decodes_golden_fields() {
    let mut data = patterned(VaultAccount::LEN);
    data[..5].copy_from_slice(&[9, 1, 255, 3, 1]);
    data[12..16].copy_from_slice(&3u32.to_le_bytes());
    data[402] = 1;
    data[403] = 0;
    let v = VaultAccount::parse(&data).unwrap();
    assert_eq!(
        (v.phase, v.safety, v.equity_computed, v.accepting_deposits),
        (
            VaultPhase::ProcessingCapital,
            VaultSafety::Frozen,
            true,
            true
        )
    );
    assert_eq!(
        format!(
            "{} {} {} {} {} {} {} {} {}",
            hex::encode(v.vault_id),
            v.governance_authority,
            v.primary_delegate,
            v.main_mint,
            v.second_mint,
            v.share_mint,
            v.total_shares,
            v.pending_deposit_count,
            v.pending_withdraw_count
        ),
        "01203f5e7d9cbbdaf91837567594b3d2f1102f4e6d8cabcae90827466584a3c2 \
         G9JqiYAjvGCTeXhpnnKXe6tRJ1fHk211DLwrBhn6gCxR \
         9j1m7vvB1HRgS4YQt5eGpCManEbsGipM5mNxNwc7w3nR \
         7ZcJYJ2SgX2xjWStJKzDVAEEEYVH6BPRdMERS76fHNmw \
         5QCqxf87tpC2WqD2hvZu1dPnrDa6ukeSztVDtjH3UteV \
         3EoPP2DnyJTnaMGb47tgNKb1HHAejnDJPXAoy1rEmakq \
         14603920617769996529 772796625 2861263949"
    );
    assert_eq!(
        (v.main_token_program, v.second_token_program),
        (TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID)
    );
    assert_eq!(
        (
            v.capital_fallback_delay_secs,
            v.closing_started_at,
            v.boundary_equity,
            v.boundary_shares
        ),
        (
            u64::from_le_bytes(data[408..416].try_into().unwrap()),
            u64::from_le_bytes(data[432..440].try_into().unwrap()),
            u64::from_le_bytes(data[440..448].try_into().unwrap()),
            u64::from_le_bytes(data[448..456].try_into().unwrap())
        )
    );

    let mut short = data.clone();
    short.pop();
    assert!(VaultAccount::parse(&short).is_err());
    let mut unknown_flag = data.clone();
    unknown_flag[12] = 4;
    assert!(VaultAccount::parse(&unknown_flag).is_err());
    data[0] = 12;
    assert!(VaultAccount::parse(&data).is_err());
}

fn vault_bytes(len: usize, version: u8) -> Vec<u8> {
    let mut data = patterned(len);
    data[..5].copy_from_slice(&[9, version, 0, 1, 0]);
    data[12..16].copy_from_slice(&1u32.to_le_bytes());
    data[402] = 0;
    data[403] = 1;
    data
}

#[test]
fn vault_account_rejects_unknown_lengths_and_mismatched_versions() {
    for len in [0, 688, 700, VaultAccount::LEN + 1] {
        assert_eq!(
            VaultAccount::parse(&vec![9; len]),
            Err(VaultChainError::AccountLength {
                expected: VaultAccount::LEN,
                actual: len
            })
        );
    }
    assert!(VaultAccount::parse(&vault_bytes(VaultAccount::LEN, 1)).is_ok());
    assert_eq!(
        VaultAccount::parse(&vault_bytes(VaultAccount::LEN, 2)),
        Err(VaultChainError::Version(2))
    );
    assert_eq!(
        VaultAccount::parse(&vault_bytes(VaultAccount::LEN, 3)),
        Err(VaultChainError::Version(3))
    );
}

#[test]
fn request_and_oracle_source_decode_golden_fields() {
    for kind in [VaultRequestKind::Deposit, VaultRequestKind::Withdraw] {
        let mut data = patterned(VaultRequest::LEN);
        data[..2].copy_from_slice(&[kind.discriminator(), 1]);
        let r = VaultRequest::parse(&data, kind).unwrap();
        assert_eq!(
            format!("{} {} {} {}", r.user, r.vault, r.amount, r.created_at),
            "ckHwo3Yn9LMEBGQax4dPKcJv95ZyqpvRTqKrCfWzcrD \
             GgfCrwtpeeaGfy2h9rWhjQXyf3H3RDXGSGqomi7NU6G9 \
             11710313704246995145 11131592321559158977"
        );
        for version in [2, 3] {
            data[1] = version;
            assert!(VaultRequest::parse(&data, kind).is_err());
        }
    }
    let mut data = patterned(424);
    data[..2].copy_from_slice(&[7, 1]);
    assert_eq!(
        hex::encode(oracle_source_pyth_feed(&data).unwrap()),
        "c9e80726456483a2c1e0ff1e3d5c7b9ab9d8f71635547392b1d0ef0e2d4c6b8a"
    );
    data[72..104].fill(0);
    assert!(oracle_source_pyth_feed(&data).is_err());
}

#[cfg(feature = "chain-rpc")]
#[test]
fn capital_tx_users_fit_the_legacy_packet() {
    use super::VAULT_CAPITAL_BATCH_USERS;
    use solana_compute_budget_interface::ComputeBudgetInstruction;
    use solana_sdk::transaction::Transaction;

    const PACKET_DATA_SIZE: usize = 1232;
    let m = manager(TOKEN_2022_PROGRAM_ID);
    let payer = key(1);
    let wire_len = |capital: Instruction| {
        let tx = Transaction::new_with_payer(
            &[
                ComputeBudgetInstruction::set_compute_unit_limit(400_000),
                capital,
            ],
            Some(&payer),
        );
        1 + tx.signatures.len() * 64 + tx.message.serialize().len()
    };
    let users = users(u8::try_from(VAULT_CAPITAL_BATCH_USERS + 1).unwrap());
    let (batch, extra) = users.split_at(VAULT_CAPITAL_BATCH_USERS);
    let oversized = |mut capital: Instruction, one: Instruction, header: usize| {
        capital.accounts.extend_from_slice(&one.accounts[header..]);
        capital
    };
    let deposits = m.process_deposits(batch).unwrap();
    let withdrawals = m.process_withdrawals(batch).unwrap();
    assert!(wire_len(deposits.clone()) <= PACKET_DATA_SIZE);
    assert!(wire_len(withdrawals.clone()) <= PACKET_DATA_SIZE);
    let deposits = oversized(deposits, m.process_deposits(extra).unwrap(), 11);
    let withdrawals = oversized(withdrawals, m.process_withdrawals(extra).unwrap(), 10);
    assert!(wire_len(deposits) > PACKET_DATA_SIZE);
    assert!(wire_len(withdrawals) > PACKET_DATA_SIZE);
}
