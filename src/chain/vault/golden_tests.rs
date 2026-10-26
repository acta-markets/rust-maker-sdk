//! Golden bytes recorded from the contract's `acta-contract-sdk`.

use sha2::{Digest, Sha256};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::{
    DepositPayout, MAX_VAULT_SWAP_PATH_ACCOUNTS, VaultAccount, VaultChainError,
    VaultGovernanceUpdate, VaultGovernanceUpdateKind, VaultManager, VaultManagerArgs, VaultPhase,
    VaultPolicyUpdate, VaultPositionRef, VaultRequest, VaultRequestKind, VaultSafety,
    VaultStagingAsset, VaultSwapAmountOffset, VaultSwapHop, VaultSwapKind, VaultSwapPlan,
    WithdrawalPayout, pda, vault_capital_batch_compute_units,
};
use crate::chain::ix::derive_associated_token_address as ata;
use crate::chain::ix::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};
use crate::{VaultDelegatePermissions, VaultLeg, VaultStagingStep};

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
        second_leg_generation: 0,
    })
    .unwrap()
}

struct Golden {
    token_program: Pubkey,
    addresses: &'static str,
    capital: [&'static str; 15],
    swaps: [&'static str; 4],
    shapes: &'static str,
    /// Conservative compute units for 1..=3 users: (deposit, withdrawal).
    all_missing: [(u32, u32); 3],
}

const GOLDEN: [Golden; 2] = [
    Golden {
        token_program: TOKEN_PROGRAM_ID,
        addresses: "83f0f330434d5981",
        capital: [
            "2c00/5/f779cd53a5637a79",
            "2c01/5/f779cd53a5637a79",
            "25/16/33bce79f49989233",
            "25/17/902221c05160ce8e",
            "28/7/f0c7f42cc0a16cbd",
            "3c/21/f93f7bc7d918f7ec",
            "3c/21/8420fd118883d1c0",
            "27/18/76f93b7e4e221903",
            "26/17/19e49dfe00549d7e",
            "27/25/50fdb4c8ec7ee764",
            "26/24/3c2076dbc29893a0",
            "27/32/82c41b8b12030e34",
            "26/31/3ac9526f9287dc5f",
            "27/25/cc1bf83688a67582",
            "26/24/5443b914191d8422",
        ],
        swaps: [
            "2d0040420f0000000000c607000000000000a4420f000000000002060400ffff3ce517cb060400ffff46e517cb/35/282fe622c1e356aa",
            "2d0140420f0000000000c607000000000000a4420f000000000001060400ffff3ce517cb/26/0fb9596be15cd2a1",
            "2e40420f0000000000c607000000000000a4420f000000000002060400ffff3ce517cb060400ffff46e517cb/32/de94b8fb0a59c128",
            "4440420f0000000000c607000000000000a4420f000000000001060400ffff3ce517cb/23/9153114763229623",
        ],
        shapes: "494335a5087e54be",
        all_missing: [(130_000, 130_000), (216_400, 209_200), (286_600, 281_200)],
    },
    Golden {
        token_program: TOKEN_2022_PROGRAM_ID,
        addresses: "6a8b6608245eb6a9",
        capital: [
            "2c00/5/f779cd53a5637a79",
            "2c01/5/f779cd53a5637a79",
            "25/16/532eaf6b4a257b60",
            "25/17/6913026b98b3ea8f",
            "28/7/7a32261d5c71ada7",
            "3c/21/9b04ca82a25313bf",
            "3c/21/2c5400de9cff0c92",
            "27/18/a790fb648e3c9a58",
            "26/17/fdbf70b09ecc621d",
            "27/25/bff1bd589080ad28",
            "26/24/a95faa21b1accbe2",
            "27/32/70752bc4a123fe16",
            "26/31/8ba5ea72877a303a",
            "27/25/9618893ffdb13bce",
            "26/24/276b0674cc3714f7",
        ],
        swaps: [
            "2d0040420f0000000000c607000000000000a4420f000000000002060400ffff3ce517cb060400ffff46e517cb/35/97b7b39e493a7aac",
            "2d0140420f0000000000c607000000000000a4420f000000000001060400ffff3ce517cb/26/7dfe82bb5afa1ada",
            "2e40420f0000000000c607000000000000a4420f000000000002060400ffff3ce517cb060400ffff46e517cb/32/cf424cdbc20aa971",
            "4440420f0000000000c607000000000000a4420f000000000001060400ffff3ce517cb/23/c819b4be7335ccd5",
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
            a.main_price_route,
            a.second_price_route,
            a.manager_stake_authority,
            a.vault_main_ata,
            a.vault_second_ata,
            a.manager_stake_ata,
        ]
        .iter()
        .flat_map(Pubkey::to_bytes)
        .collect();
        assert_eq!(digest(&addresses), golden.addresses);

        let section = price_section();
        let mut capital = vec![
            m.set_accepting_deposits(false),
            m.set_accepting_deposits(true),
            m.process_equity(&section),
            m.process_idle_withdrawal(key(12), &section),
            m.finalize_cycle(),
            m.process_final_withdrawal(key(12)),
            m.process_final_withdrawal(WithdrawalPayout::new(key(12)).shares_back_to(key(13))),
        ];
        for count in 1..=3 {
            capital.push(m.process_deposits(users(count)).unwrap());
            capital.push(m.process_withdrawals(users(count)).unwrap());
        }
        capital.push(
            m.process_deposits([
                DepositPayout::new(key(20)).refund_to(key(21)),
                DepositPayout::new(key(22)).shares_to(key(23)),
            ])
            .unwrap(),
        );
        capital.push(
            m.process_withdrawals([
                WithdrawalPayout::new(key(20)).shares_back_to(key(21)),
                WithdrawalPayout::new(key(22)),
            ])
            .unwrap(),
        );
        assert_eq!(capital.iter().map(ix).collect::<Vec<_>>(), golden.capital);
    }
}

fn price_section() -> Vec<AccountMeta> {
    [10, 11, 12]
        .map(|seed| AccountMeta::new_readonly(key(seed), false))
        .to_vec()
}

fn hop(program: u8, vault: Pubkey, source: Pubkey, destination: Pubkey, extra: u8) -> VaultSwapHop {
    VaultSwapHop {
        program: key(program),
        source,
        destination,
        accounts: vec![
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(vault, true),
            AccountMeta::new(source, false),
            AccountMeta::new(key(extra), false),
            AccountMeta::new(destination, false),
            AccountMeta::new_readonly(key(extra + 1), false),
        ],
        data: vec![extra, 0xe5, 0x17, 0xcb],
        amount_offset: None,
    }
}

#[test]
fn swap_instructions_match_golden() {
    for golden in GOLDEN {
        let m = manager(golden.token_program);
        let a = m.addresses;
        let transit = crate::chain::ix::derive_associated_token_address(
            &a.vault,
            &key(40),
            &TOKEN_PROGRAM_ID,
        );
        let plan = |input: Pubkey, output: Pubkey, two: bool| VaultSwapPlan {
            price_section: price_section(),
            amount_in: 1_000_000,
            minimum_out: 1_990,
            valid_until: 1_000_100,
            hops: if two {
                vec![
                    hop(50, a.vault, input, transit, 60),
                    hop(51, a.vault, transit, output, 70),
                ]
            } else {
                vec![hop(50, a.vault, input, output, 60)]
            },
        };
        let (main, second) = (a.vault_main_ata, a.vault_second_ata);
        let swaps = [
            (
                VaultSwapKind::Rebalance {
                    second_to_main: false,
                },
                plan(main, second, true),
            ),
            (
                VaultSwapKind::Rebalance {
                    second_to_main: true,
                },
                plan(second, main, false),
            ),
            (VaultSwapKind::Recovery, plan(second, main, true)),
            (VaultSwapKind::Closing, plan(second, main, false)),
        ]
        .map(|(kind, plan)| ix(&m.swap(kind, &plan).unwrap()));
        assert_eq!(swaps, golden.swaps);
    }
}

#[test]
fn swap_hops_must_chain_custody_and_stay_within_caps() {
    let m = manager(TOKEN_PROGRAM_ID);
    let a = m.addresses;
    let (main, second, transit) = (a.vault_main_ata, a.vault_second_ata, key(40));
    let kind = VaultSwapKind::Rebalance {
        second_to_main: false,
    };
    let swap = |hops: Vec<VaultSwapHop>| {
        m.swap(
            kind.clone(),
            &VaultSwapPlan {
                price_section: price_section(),
                amount_in: 1,
                minimum_out: 1,
                valid_until: 1,
                hops,
            },
        )
    };
    assert!(swap(vec![hop(50, a.vault, main, second, 60)]).is_ok());
    for broken in [
        vec![],
        vec![hop(50, a.vault, second, main, 60)],
        vec![hop(50, a.vault, main, transit, 60)],
        vec![
            hop(50, a.vault, main, transit, 60),
            hop(51, a.vault, main, second, 70),
        ],
        vec![
            hop(50, a.vault, main, second, 60),
            hop(51, a.vault, second, second, 70),
        ],
        vec![
            hop(50, a.vault, main, transit, 60),
            hop(51, a.vault, transit, transit, 70),
            hop(52, a.vault, transit, transit, 80),
            hop(53, a.vault, transit, transit, 90),
            hop(54, a.vault, transit, second, 100),
        ],
    ] {
        assert!(swap(broken).is_err());
    }
    let mut signer = hop(50, a.vault, main, second, 60);
    signer
        .accounts
        .push(AccountMeta::new_readonly(key(99), true));
    assert!(swap(vec![signer]).is_err());
    let mut no_vault = hop(50, a.vault, main, second, 60);
    no_vault.accounts.remove(1);
    assert!(swap(vec![no_vault]).is_err());
    let mut wide = hop(50, a.vault, main, second, 60);
    wide.accounts
        .extend((0..59).map(|_| AccountMeta::new_readonly(Pubkey::new_unique(), false)));
    assert!(swap(vec![wide]).is_err());
    let mut long = hop(50, a.vault, main, second, 60);
    long.data = vec![1; 1_025];
    assert!(swap(vec![long]).is_err());
}

#[test]
fn hop_header_carries_the_amount_offset() {
    let m = manager(TOKEN_PROGRAM_ID);
    let a = m.addresses;
    let (main, second, transit) = (a.vault_main_ata, a.vault_second_ata, key(40));
    let mut hops = vec![
        hop(50, a.vault, main, transit, 60),
        hop(51, a.vault, transit, second, 70),
    ];
    hops[1].data = (0..9).collect();
    hops[1].amount_offset = Some(VaultSwapAmountOffset {
        offset: 1,
        num: 3,
        den: 2,
    });
    let swap = m
        .swap(
            VaultSwapKind::Rebalance {
                second_to_main: false,
            },
            &VaultSwapPlan {
                price_section: price_section(),
                amount_in: 1,
                minimum_out: 1,
                valid_until: 1,
                hops,
            },
        )
        .unwrap();
    assert_eq!(swap.data[26], 2);
    assert_eq!(
        &swap.data[27..36],
        &[6, 4, 0, 0xff, 0xff, 60, 0xe5, 0x17, 0xcb]
    );
    assert_eq!(&swap.data[36..41], &[6, 9, 0, 1, 0]);
    assert_eq!(&swap.data[41..49], &3u64.to_le_bytes());
    assert_eq!(&swap.data[49..57], &2u64.to_le_bytes());
    assert_eq!(&swap.data[57..], &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn staging_swap_layout_and_staging_mint() {
    let m = manager(TOKEN_PROGRAM_ID);
    let a = m.addresses;
    let asset = VaultStagingAsset {
        mint: key(41),
        token_program: TOKEN_2022_PROGRAM_ID,
    };
    let staging_ata = asset.vault_ata(&a.vault);
    let kind = |step, leg, asset| VaultSwapKind::Staging {
        step,
        leg,
        asset,
        generation: 0x0102_0304_0506_0708,
    };
    let plan = |input, output| VaultSwapPlan {
        price_section: price_section(),
        amount_in: 1_000_000,
        minimum_out: 1_990,
        valid_until: 1_000_100,
        hops: vec![hop(50, a.vault, input, output, 60)],
    };
    let unstage = m
        .swap(
            kind(VaultStagingStep::Unstage, VaultLeg::Second, asset),
            &plan(staging_ata, a.vault_second_ata),
        )
        .unwrap();
    assert_eq!(&unstage.data[..3], &[76, 1, 1]);
    assert_eq!(&unstage.data[3..11], &1_000_000u64.to_le_bytes());
    assert_eq!(
        &unstage.data[27..35],
        &0x0102_0304_0506_0708u64.to_le_bytes()
    );
    assert_eq!(unstage.data[35], 1);
    assert_eq!(&unstage.data[36..41], &[6, 4, 0, 0xff, 0xff]);
    let fixed: Vec<_> = unstage.accounts[..14]
        .iter()
        .map(|meta| (meta.pubkey, meta.is_signer, meta.is_writable))
        .collect();
    assert_eq!(
        fixed,
        [
            (key(1), true, false),
            (a.vault, false, true),
            (a.policy, false, false),
            (a.delegate, false, false),
            (a.global_config, false, false),
            (a.vault_second_ata, false, true),
            (key(4), false, false),
            (TOKEN_PROGRAM_ID, false, false),
            (staging_ata, false, true),
            (key(41), false, false),
            (TOKEN_2022_PROGRAM_ID, false, false),
            (key(5), false, false),
            (a.manager_stake_ata, false, false),
            (a.swap_venues, false, false),
        ]
    );
    let stage = kind(VaultStagingStep::Stage, VaultLeg::Main, asset);
    let swap = m
        .swap(stage.clone(), &plan(a.vault_main_ata, staging_ata))
        .unwrap();
    assert_eq!(&swap.data[..3], &[76, 0, 0]);
    for (kind, input, output) in [
        (stage.clone(), staging_ata, a.vault_main_ata),
        (stage, a.vault_main_ata, a.vault_second_ata),
        (
            kind(VaultStagingStep::Stage, VaultLeg::Unknown, asset),
            a.vault_main_ata,
            staging_ata,
        ),
        (
            kind(
                VaultStagingStep::Stage,
                VaultLeg::Main,
                VaultStagingAsset {
                    mint: key(4),
                    token_program: TOKEN_PROGRAM_ID,
                },
            ),
            a.vault_main_ata,
            a.vault_second_ata,
        ),
        (
            kind(
                VaultStagingStep::Stage,
                VaultLeg::Main,
                VaultStagingAsset {
                    mint: key(41),
                    token_program: key(42),
                },
            ),
            a.vault_main_ata,
            ata(&a.vault, &key(41), &key(42)),
        ),
    ] {
        assert!(m.swap(kind, &plan(input, output)).is_err());
    }
}

#[test]
fn amount_offset_only_on_a_transit_source_and_inside_the_data() {
    let m = manager(TOKEN_PROGRAM_ID);
    let a = m.addresses;
    let (main, second, transit) = (a.vault_main_ata, a.vault_second_ata, key(40));
    for (index, offset, num, den, ok) in [
        (1, 9, 1, 1, true),
        (1, 0, 1_081_430_937_307, 1_000_000_000_000, true),
        (1, 10, 1, 1, false),
        (1, u16::MAX, 1, 1, false),
        (1, 0, 0, 1, false),
        (1, 0, 1, 0, false),
        (0, 0, 1, 1, false),
    ] {
        let mut hops = vec![
            hop(50, a.vault, main, transit, 60),
            hop(51, a.vault, transit, second, 70),
        ];
        hops[0].data = vec![0; 17];
        hops[1].data = vec![0; 17];
        hops[index].amount_offset = Some(VaultSwapAmountOffset { offset, num, den });
        let plan = VaultSwapPlan {
            price_section: price_section(),
            amount_in: 1,
            minimum_out: 1,
            valid_until: 1,
            hops,
        };
        let kind = VaultSwapKind::Rebalance {
            second_to_main: false,
        };
        assert_eq!(
            m.swap(kind, &plan).is_ok(),
            ok,
            "hop {index} offset {offset}"
        );
    }
}

#[test]
fn swap_accounts_cap_per_hop_and_across_the_path() {
    let m = manager(TOKEN_PROGRAM_ID);
    let a = m.addresses;
    let (main, second, transit) = (a.vault_main_ata, a.vault_second_ata, key(40));
    let pad = AccountMeta::new_readonly(key(99), false);
    let sized = |sizes: [usize; 2]| {
        let mut hops = vec![
            hop(50, a.vault, main, transit, 60),
            hop(51, a.vault, transit, second, 70),
        ];
        for (hop, size) in hops.iter_mut().zip(sizes) {
            hop.accounts.resize(size, pad.clone());
        }
        let plan = VaultSwapPlan {
            price_section: price_section(),
            amount_in: 1,
            minimum_out: 1,
            valid_until: 1,
            hops,
        };
        m.swap(
            VaultSwapKind::Rebalance {
                second_to_main: false,
            },
            &plan,
        )
        .is_ok()
    };
    assert_eq!(MAX_VAULT_SWAP_PATH_ACCOUNTS, 128);
    assert!(sized([40, 34]));
    assert!(sized([64, 58]));
    assert!(!sized([64, 59]));
    assert!(!sized([65, 6]));
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
                    m.deposit_batch_shape(&users, missing, rest),
                    m.withdrawal_batch_shape(&users, missing, rest),
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
                    vault_capital_batch_compute_units(m.deposit_batch_shape(
                        &users,
                        users.len(),
                        users.len(),
                    )),
                    vault_capital_batch_compute_units(m.withdrawal_batch_shape(
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
        pda::price_route(&program, &user),
        pda::swap_venues(&program),
    ]
    .iter()
    .flat_map(|(address, bump)| address.to_bytes().into_iter().chain([*bump]))
    .collect();
    assert_eq!(digest(&pdas), "4ac0d11bb5d2de1b");
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
    let section = price_section();
    let prices = Some(section.as_slice());
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
            "3700fcffffffffffffff000000000000000000000000000000000000000000000000/5/5ec5dd1abd480928",
            "3701d007c80000000000000000000000000000000000000000000000000000000000/5/45c03854ef6a24bc",
            "3702f6ffffffffffffffffffffffffffffff00000000000000000000000000000000/5/87e39b3943882b86",
            "37030500000007000000000000000000000000000000000000000000000000000000/5/8e0cd501f9328fa8",
            "370403009600c4090000000000000000000000000000000000000000000000000000/5/48194846a6c25808",
            "37051e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e/5/9104fa0231f4964e",
            "3800/4/d0371903118ce95d",
            "3900/4/cd7fafcea90fbe03",
            "3801/17/d0bbcab7b62b8dc5",
            "3901/4/45147bb440919f0e",
            "3802/17/43a963367ed55b4a",
            "3902/4/a48478635846532e",
            "3803/4/dc0663ae22446adb",
            "3903/4/599a443650ae290a",
            "3804/4/142623051969a573",
            "3904/4/7f29ff5418d81c11",
            "3805/4/da408c2b2eb99f79",
            "3905/4/1008c0966018528c",
            "2f20202020202020202020202020202020202020202020202020202020202020203f00000000b33f7100000000/5/db66018087f022ff",
            "3e/3/4a35ed45ecc4ab38",
        ]
    );
    assert_eq!(unpriced, [true, false, false, true, true, true]);
    assert!(
        m.rotate_delegate(key(32), VaultDelegatePermissions::from_bits(1 << 6), 0)
            .is_err()
    );
}

#[test]
fn second_leg_roll_applies_through_its_own_builder_and_refreshes_the_manager() {
    let mut m = manager(TOKEN_2022_PROGRAM_ID);
    let args = m.args;
    let new_mint = key(40);
    let update = VaultGovernanceUpdate::SecondLeg(new_mint);
    assert_eq!(update.kind() as u8, 6);
    assert_eq!(update.payload().unwrap(), new_mint.to_bytes());
    assert!(
        VaultGovernanceUpdate::SecondLeg(Pubkey::default())
            .payload()
            .is_err()
    );
    assert!(m.apply_update(update.kind(), args.actor, None).is_err());
    for mint in [args.main_mint, args.second_mint, args.share_mint] {
        assert!(m.apply_second_leg(args.actor, mint, None).is_err());
    }

    let ix = m.apply_second_leg(key(31), new_mint, None).unwrap();
    let a = m.addresses;
    assert_eq!(ix.data, [56, 6]);
    let keys: Vec<_> = ix.accounts.iter().map(|meta| meta.pubkey).collect();
    assert_eq!(
        keys,
        [
            a.vault,
            a.policy,
            pda::pending_vault_update(&args.program_id, &a.vault, 6).0,
            key(31),
            args.actor,
            args.second_mint,
            a.vault_second_ata,
            TOKEN_PROGRAM_ID,
            new_mint,
            ata(&a.vault, &new_mint, &TOKEN_PROGRAM_ID),
            TOKEN_PROGRAM_ID,
            pda::price_route(&args.program_id, &new_mint).0,
            a.second_price_route,
            crate::chain::ix::SYSTEM_PROGRAM_ID,
            crate::chain::ix::ASSOCIATED_TOKEN_PROGRAM_ID,
            a.vault_main_ata,
        ]
    );
    let writable: Vec<_> = ix.accounts.iter().map(|meta| meta.is_writable).collect();
    assert_eq!(
        writable,
        [
            true, true, true, true, true, true, true, false, false, true, false, false, false,
            false, false, false
        ]
    );
    assert!(
        ix.accounts
            .iter()
            .enumerate()
            .all(|(i, meta)| meta.is_signer == (i == 4))
    );
    let section = price_section();
    let priced = m
        .apply_second_leg(key(31), new_mint, Some(&section))
        .unwrap();
    assert_eq!(&priced.accounts[16..], &section[..]);

    let mut account = m.test_account(Default::default());
    assert_eq!(m.ensure_current(&account), Ok(()));
    assert_eq!(m.refresh(&account), Ok(false));
    account.second_leg_generation = 1;
    account.second_mint = new_mint;
    assert_eq!(
        m.ensure_current(&account),
        Err(VaultChainError::SecondLegRolled)
    );
    let stale = m;
    assert_eq!(m.refresh(&account), Ok(true));
    assert_eq!(m.ensure_current(&account), Ok(()));
    assert_eq!(m.args.second_leg_generation, 1);
    assert_eq!(
        m.addresses.vault_second_ata,
        ata(&a.vault, &new_mint, &TOKEN_PROGRAM_ID)
    );
    assert_eq!(
        m.addresses.second_price_route,
        pda::price_route(&args.program_id, &new_mint).0
    );
    assert_eq!(
        (m.addresses.vault_main_ata, m.addresses.delegate),
        (a.vault_main_ata, a.delegate)
    );
    assert_ne!(stale, m);
    account.vault_id = [8; 32];
    assert!(matches!(
        m.refresh(&account),
        Err(VaultChainError::InvalidInput(_))
    ));
}

fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index * 31 + 17) as u8).collect()
}

#[test]
fn vault_account_decodes_golden_fields() {
    let mut data = patterned(VaultAccount::LEN);
    data[..7].copy_from_slice(&[9, 1, 255, 3, 1, 1, 0]);
    data[12..16].copy_from_slice(&3u32.to_le_bytes());
    data[402] = 1;
    data[403] = 0;
    let v = VaultAccount::parse(&data).unwrap();
    assert_eq!(super::VaultView::from(&v).staging, v.staging);
    let slot = v.staging.open.unwrap();
    assert_eq!(
        (
            slot.mint,
            slot.from_leg,
            slot.amount,
            slot.staged_at,
            v.staging.generation
        ),
        (
            Pubkey::new_from_array(data[456..488].try_into().unwrap()),
            VaultLeg::Second,
            u64::from_le_bytes(data[488..496].try_into().unwrap()),
            u64::from_le_bytes(data[496..504].try_into().unwrap()),
            u64::from_le_bytes(data[504..512].try_into().unwrap())
        )
    );
    let mut empty = data.clone();
    empty[488..496].fill(0);
    empty[5] = 7;
    let empty = VaultAccount::parse(&empty).unwrap().staging;
    assert_eq!(empty.open, None);
    assert_eq!(empty.generation, v.staging.generation);
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
    assert_eq!(
        v.second_leg_generation,
        u64::from_le_bytes(data[512..520].try_into().unwrap())
    );

    let mut short = data.clone();
    short.pop();
    assert!(VaultAccount::parse(&short).is_err());
    let mut unknown_flag = data.clone();
    unknown_flag[12] = 4;
    assert!(VaultAccount::parse(&unknown_flag).is_err());
    let mut staged_kind = data.clone();
    staged_kind[6] = 1;
    assert_eq!(
        VaultAccount::parse(&staged_kind),
        Err(VaultChainError::UnknownStagedKind(1))
    );
    let mut staged_leg = data.clone();
    staged_leg[5] = 2;
    assert_eq!(
        VaultAccount::parse(&staged_leg),
        Err(VaultChainError::UnknownStagedLeg(2))
    );
    data[0] = 12;
    assert!(VaultAccount::parse(&data).is_err());
}

fn vault_bytes(len: usize, version: u8) -> Vec<u8> {
    let mut data = patterned(len);
    data[..7].copy_from_slice(&[9, version, 0, 1, 0, 0, 0]);
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
fn request_decodes_golden_fields() {
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

#[test]
fn share_return_account_matches_create_with_seed() {
    let m = manager(TOKEN_PROGRAM_ID);
    let user = key(20);
    for kind in [VaultRequestKind::Deposit, VaultRequestKind::Withdraw] {
        let request = match kind {
            VaultRequestKind::Deposit => {
                super::pda::deposit_request(&m.args.program_id, &m.addresses.vault, &user)
            }
            VaultRequestKind::Withdraw => {
                super::pda::withdraw_request(&m.args.program_id, &m.addresses.vault, &user)
            }
        }
        .0;
        let digest = Sha256::new()
            .chain_update(b"acta-share-return-v1")
            .chain_update(request)
            .finalize();
        let expected = Pubkey::create_with_seed(
            &m.args.actor,
            &hex::encode(&digest[..16]),
            &TOKEN_PROGRAM_ID,
        )
        .unwrap();
        assert_eq!(m.share_return_account(kind, &user), expected);
    }
}
