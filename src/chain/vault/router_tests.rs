use super::router::*;
use super::venues::{SwapVenues, exponent::*, jupiter};
use super::*;
use crate::chain::ix::TOKEN_PROGRAM_ID;
use crate::types::{VaultLeg, VaultStagingStep};
use solana_sdk::{instruction::AccountMeta, message::AddressLookupTableAccount, pubkey::Pubkey};
fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn put(data: &mut [u8], at: usize, k: Pubkey) {
    data[at..at + 32].copy_from_slice(k.as_ref());
}
fn market() -> Market {
    let mut d = vec![0; 1620];
    d[..8].copy_from_slice(&[242, 240, 26, 15, 148, 186, 185, 205]);
    for (i, k) in [
        key(9),
        key(3),
        key(4),
        key(5),
        key(6),
        key(7),
        key(8),
        key(10),
        key(11),
        key(12),
        key(13),
        key(14),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut d, 40 + i * 32, k);
    }
    put(&mut d, 426, GENERIC_SY);
    Market::parse(key(13), &d).unwrap()
}
fn sy() -> GenericSy {
    let mut d = vec![0; 240];
    d[..8].copy_from_slice(&[254, 147, 136, 16, 163, 203, 98, 93]);
    put(&mut d, 8, key(4));
    d[97..105].copy_from_slice(&1_080_000_000_000u64.to_le_bytes());
    put(&mut d, 129, key(15));
    d[161] = 1;
    d[204..208].copy_from_slice(&1u32.to_le_bytes());
    put(&mut d, 208, key(16));
    d.extend([0; 4]);
    GenericSy::parse(key(17), &d).unwrap()
}
fn core() -> CoreVault {
    let mut d = vec![0; 548];
    d[..8].copy_from_slice(&[211, 8, 232, 43, 2, 152, 117, 119]);
    for (i, k) in [
        GENERIC_SY,
        key(4),
        key(5),
        key(3),
        key(18),
        key(19),
        key(20),
        key(21),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut d, 8 + i * 32, k);
    }
    d[264..268].copy_from_slice(&100u32.to_le_bytes());
    d[268..272].copy_from_slice(&100u32.to_le_bytes());
    put(&mut d, 304, key(22));
    CoreVault::parse(key(6), &d).unwrap()
}
fn registry() -> SwapVenues {
    let mut d = vec![0; 3364];
    d[0] = 16;
    d[1] = 1;
    d[3] = 4;
    for (i, (program, selectors)) in [
        (
            jupiter::PROGRAM,
            vec![jupiter::EXACT_IN_SELECTORS[0].to_vec()],
        ),
        (GENERIC_SY, vec![vec![1], vec![2]]),
        (CLMM, vec![vec![3]]),
        (CORE, vec![vec![5]]),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 4 + 105 * i;
        put(&mut d, at, program);
        d[at + 32] = selectors.len() as u8;
        for (j, s) in selectors.into_iter().enumerate() {
            let at = at + 33 + j * 9;
            d[at] = s.len() as u8;
            d[at + 1..at + 1 + s.len()].copy_from_slice(&s);
        }
    }
    SwapVenues::parse(&d).unwrap()
}
fn route(mint: Pubkey) -> PriceRoute {
    PriceRoute {
        mint,
        max_skew_secs: 10,
        updated_slot: 0,
        bricks: vec![PriceBrick::Pyth(PythBrick {
            feed_id: mint.to_bytes(),
            max_age_secs: 60,
            min_band_ppm: 0,
        })],
    }
}
pub(super) fn manager() -> VaultManager {
    VaultManager::new(VaultManagerArgs {
        program_id: key(1),
        actor: key(23),
        vault_id: [24; 32],
        delegate_authority: key(23),
        main_mint: key(2),
        second_mint: key(3),
        share_mint: key(25),
        main_token_program: TOKEN_PROGRAM_ID,
        second_token_program: TOKEN_PROGRAM_ID,
        second_leg_generation: 0,
    })
    .unwrap()
}
pub(super) fn quote(manager: &VaultManager, buy: bool, extra: usize) -> jupiter::Quote {
    let owner = manager.addresses.vault;
    let base =
        crate::chain::ix::derive_associated_token_address(&owner, &key(15), &TOKEN_PROGRAM_ID);
    let (source, destination) = if buy {
        (manager.addresses.vault_main_ata, base)
    } else {
        (base, manager.addresses.vault_main_ata)
    };
    let mut accounts = vec![
        AccountMeta::new(owner, true),
        AccountMeta::new(source, false),
        AccountMeta::new(destination, false),
    ];
    accounts.extend((0..extra).map(|i| AccountMeta::new(key(100 + i as u8), false)));
    let mut data = jupiter::EXACT_IN_SELECTORS[0].to_vec();
    data.extend(0u32.to_le_bytes());
    data.extend(1000u64.to_le_bytes());
    data.extend(1000u64.to_le_bytes());
    data.extend([50, 0, 0]);
    jupiter::Quote {
        request: jupiter::Request {
            owner,
            payer: manager.args.actor,
            input_mint: if buy { key(2) } else { key(15) },
            output_mint: if buy { key(15) } else { key(2) },
            source,
            destination,
            amount: 1000,
            slippage_bps: 50,
        },
        output: 1000,
        minimum: 995,
        hop: VaultSwapHop {
            program: jupiter::PROGRAM,
            source,
            destination,
            accounts,
            data,
            amount_offset: None,
        },
        tables: vec![],
    }
}
pub(super) fn run(
    intent: Intent,
    extra: usize,
    slot: Option<VaultStagedSlot>,
    now: u64,
) -> Result<Plan, VaultChainError> {
    run_with_quote(intent, extra, slot, now, |_| {})
}
fn run_with_quote(
    intent: Intent,
    extra: usize,
    slot: Option<VaultStagedSlot>,
    now: u64,
    mutate: impl FnOnce(&mut jupiter::Quote),
) -> Result<Plan, VaultChainError> {
    let manager = manager();
    let mut account = manager.test_account(VaultStagingState {
        generation: if slot.is_some() { 8 } else { 7 },
        open: slot,
    });
    account.phase = VaultPhase::Active;
    account.active_expiry = 1000;
    let m = market();
    let sy = sy();
    let core = core();
    let registry = registry();
    let routes = [route(key(2)), route(key(3)), route(key(15))];
    let mut tables = vec![
        AddressLookupTableAccount {
            key: m.lookup_table,
            addresses: vec![],
        },
        AddressLookupTableAccount {
            key: core.lookup_table,
            addresses: vec![],
        },
    ];
    let buy = super::router::recovery_buy(&account, intent, now, core.maturity)?;
    let mut q = quote(&manager, buy, extra);
    mutate(&mut q);
    // A complete test ALT compresses bytes while preserving account-lock pressure.
    let mut addresses: Vec<_> = (1..200).map(key).collect();
    addresses.extend([
        manager.addresses.vault,
        manager.addresses.policy,
        manager.addresses.delegate,
        manager.addresses.global_config,
        manager.addresses.swap_venues,
        manager.addresses.manager_stake_ata,
        manager.addresses.vault_main_ata,
        manager.addresses.vault_second_ata,
    ]);
    for mint in [key(4), key(5), key(15)] {
        addresses.push(crate::chain::ix::derive_associated_token_address(
            &manager.addresses.vault,
            &mint,
            &TOKEN_PROGRAM_ID,
        ));
    }
    for r in &routes {
        addresses.extend(
            r.accounts(pda::price_route(&key(1), &r.mint).0, 0)
                .iter()
                .map(|a| a.pubkey),
        );
    }
    addresses.push(crate::chain::ix::derive_associated_token_address(
        &sy.address,
        &sy.base,
        &TOKEN_PROGRAM_ID,
    ));
    addresses.push(Pubkey::find_program_address(&[b"__event_authority"], &CLMM).0);
    addresses.push(Pubkey::find_program_address(&[b"__event_authority"], &CORE).0);
    tables.push(AddressLookupTableAccount {
        key: key(250),
        addresses,
    });
    Router {
        manager: &manager,
        account: &account,
        main: &routes[0],
        second: &routes[1],
        staging: &routes[2],
        registry: &registry,
        market: &m,
        sy: &sy,
        core: &core,
        tables: &tables,
        shard: 0,
        now,
        valid_until: now + 60,
        base_program: TOKEN_PROGRAM_ID,
        setup: vec![],
        prices: [key(2), key(3), key(15)].map(|mint| OraclePrice {
            mint,
            usd: 1.0,
            decimals: 9,
        }),
    }
    .plan(
        intent,
        if slot.is_some() && buy {
            None
        } else {
            Some(&q)
        },
        990,
        990,
    )
}
#[test]
fn adapters_layout_and_maturity() {
    let sy = sy();
    let owner = key(30);
    let hop = sy.hop(owner, TOKEN_PROGRAM_ID, 1000, true, true).unwrap();
    assert_eq!(hop.data[0], 1);
    assert_eq!(hop.amount_offset.unwrap().num, 1_080_000_000_000);
    assert_eq!(sy.mint_amount(1000).unwrap(), 1081);
    assert_eq!(
        sy.hop(owner, TOKEN_PROGRAM_ID, 1000, false, false)
            .unwrap()
            .data,
        [vec![2], 1000u64.to_le_bytes().to_vec()].concat()
    );
    let market = market();
    assert_eq!(
        market.trade(owner, 123, false, &[], false).unwrap().data,
        [vec![3], 123u64.to_le_bytes().to_vec(), vec![0, 0, 0]].concat()
    );
    let core = core();
    assert!(core.merge(owner, 100, 200, &[]).is_err());
    let merge = core.merge(owner, 100, 201, &[]).unwrap();
    assert_eq!(
        merge.data,
        [vec![5], 100u64.to_le_bytes().to_vec()].concat()
    );
    assert_eq!(
        merge.accounts[5].pubkey,
        crate::chain::ix::derive_associated_token_address(&owner, &key(5), &TOKEN_PROGRAM_ID)
    );
}
#[test]
fn planner_buy_sell_redeem_and_open_slot() {
    assert!(matches!(
        run(Intent::Buy(1000), 0, None, 100).unwrap(),
        Plan::Single(_)
    ));
    assert!(matches!(
        run(Intent::Sell(1000), 0, None, 100).unwrap(),
        Plan::Single(_)
    ));
    assert!(run(Intent::Redeem(1000), 0, None, 200).is_err());
    assert!(run(Intent::Redeem(1000), 0, None, 201).is_ok());
    for now in [200, 201] {
        assert_eq!(
            run(Intent::Buy(1000), 0, None, now).unwrap_err(),
            VaultChainError::InvalidInput("cannot buy matured PT")
        );
    }
    let slot = VaultStagedSlot {
        mint: key(15),
        from_leg: VaultLeg::Main,
        amount: 1000,
        staged_at: 99,
    };
    let plan = run(Intent::Buy(99), 0, Some(slot), 100).unwrap();
    assert!(matches!(
        plan,
        Plan::Single(TxParts {
            kind: VaultSwapKind::Staging {
                step: VaultStagingStep::Unstage,
                generation: 8,
                ..
            },
            ..
        })
    ));
    let tx = plan.first().instructions.last().unwrap();
    assert_eq!(u64::from_le_bytes(tx.data[3..11].try_into().unwrap()), 1000);
}
#[test]
fn planner_stages_when_combined_locks_exceed_limit() {
    let plan = (1..45)
        .find_map(|n| match run(Intent::Buy(1000), n, None, 100) {
            Ok(p @ Plan::Staged { .. }) => Some(p),
            _ => None,
        })
        .expect("some route should need staging");
    let Plan::Staged { stage, unstage } = plan else {
        panic!()
    };
    assert!(stage.fit(manager().args.actor).unwrap().fits());
    assert!(unstage.fit(manager().args.actor).unwrap().fits());
    assert!(matches!(
        stage.kind,
        VaultSwapKind::Staging {
            generation: 8,
            step: VaultStagingStep::Stage,
            ..
        }
    ));
}
#[test]
fn oracle_one_percent_boundary() {
    let p = OraclePrice {
        mint: key(1),
        usd: 1.0,
        decimals: 6,
    };
    assert!(p.check_quote(10000, p, 9900).is_ok());
    assert!(p.check_quote(10000, p, 9899).is_err());
}
#[test]
fn mainnet_account_decoders() {
    let market = Market::parse(
        "8JCVSxVrM1f5stBwNWko2D5Yt9p4wJQsvPVWFZBkzhmo"
            .parse()
            .unwrap(),
        include_bytes!("../../../tests/curator_data/market.bin"),
    )
    .unwrap();
    let sy = GenericSy::parse(
        "9QReDA3ndt8FBQVJHGzM8h6NKojqyh2Ej8cqFn9dY6Hn"
            .parse()
            .unwrap(),
        include_bytes!("../../../tests/curator_data/sy.bin"),
    )
    .unwrap();
    let core = CoreVault::parse(
        "Ciz4yqREYbMH4dtQvB3Rc2mSVEGMs8jFfE97qj6nXAPu"
            .parse()
            .unwrap(),
        include_bytes!("../../../tests/curator_data/core.bin"),
    )
    .unwrap();
    assert_eq!(market.pt, core.pt);
    assert_eq!(market.sy, sy.mint);
    assert_eq!(core.sy, sy.mint);
    assert_eq!(core.address, market.vault);
    assert!(sy.index > INDEX_ONE);
    assert!(core.maturity > 1_700_000_000);
    assert_eq!(
        sy.base.to_string(),
        "hy1oXYgrBW6PVcJ4s6s2FKavRdwgWTXdfE69AxT7kPT"
    );
}
#[test]
fn funding_current_cycle_held_itm_only() {
    use crate::chain::{MarketInfo, PositionInfo, PositionStatus};
    use crate::types::{Decimals, OrderId, Quantity};
    let manager = manager();
    let mut vault = manager.test_account(VaultStagingState::default());
    vault.active_expiry = 200;
    vault.phase = VaultPhase::Active;
    let market = MarketInfo {
        settlement_price: 0,
        expiry_ts: 200,
        is_put: false,
        is_finalized: false,
        underlying_mint: vault.second_mint,
        quote_mint: vault.main_mint,
        underlying_token_program_id: TOKEN_PROGRAM_ID,
        quote_token_program_id: TOKEN_PROGRAM_ID,
        underlying_decimals: Decimals::new(9),
        quote_decimals: Decimals::new(6),
    };
    let mut position = PositionInfo {
        terms: crate::OptionTerms::CoveredCall {
            strike: crate::Strike::new(100_000_000_000),
        },
        side: crate::TakerSide::Sell,
        flags: 0,
        flags2: 0,
        flags3: 0,
        status: PositionStatus::Open,
        taker_owner: key(90),
        maker_owner: manager.addresses.vault,
        market_pda: key(91),
        quantity: Quantity::new(2_000_000_000),
        order_id: OrderId::new([1; 32]),
    };
    let markets = [FundingMarket {
        address: key(91),
        market: &market,
        price: 110_000_000_000,
    }];
    let needs = funding_needs(
        &vault,
        manager.addresses.vault,
        &[position.clone()],
        &markets,
        199,
    )
    .unwrap();
    assert_eq!(needs.main, 200_000_000);
    assert!(needs.can_acquire_second);
    assert!(
        !funding_needs(
            &vault,
            manager.addresses.vault,
            &[position.clone()],
            &markets,
            200
        )
        .unwrap()
        .can_acquire_second
    );
    position.status = PositionStatus::Funded;
    assert_eq!(
        funding_needs(&vault, manager.addresses.vault, &[position], &markets, 201)
            .unwrap()
            .main,
        0
    );
}
#[test]
fn golden_served_pt_hop_bytes() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/vault_served_tx.json")).unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["swap"]["hops"] == 3)
        .unwrap();
    let tx = STANDARD
        .decode(case["served"]["tx_base64"].as_str().unwrap())
        .unwrap();
    fn short(data: &[u8], at: &mut usize) -> usize {
        let mut result = 0;
        let mut shift = 0;
        loop {
            let b = data[*at];
            *at += 1;
            result |= usize::from(b & 127) << shift;
            if b < 128 {
                return result;
            }
            shift += 7;
        }
    }
    let mut at = 0;
    let signatures = short(&tx, &mut at);
    at += signatures * 64;
    assert_eq!(tx[at], 128);
    at += 4;
    let keys = short(&tx, &mut at);
    at += 32 * keys + 32;
    let count = short(&tx, &mut at);
    let mut swap = None;
    for _ in 0..count {
        at += 1;
        let accounts = short(&tx, &mut at);
        at += accounts;
        let len = short(&tx, &mut at);
        let data = &tx[at..at + len];
        if data.first() == Some(&45) {
            swap = Some(data);
        }
        at += len;
    }
    let swap = swap.unwrap();
    assert_eq!(swap[26], 3);
    let mut at = 27;
    let mut hops = Vec::new();
    for _ in 0..3 {
        let len = u16::from_le_bytes(swap[at + 1..at + 3].try_into().unwrap()) as usize;
        let offset = u16::from_le_bytes(swap[at + 3..at + 5].try_into().unwrap());
        at += 5;
        let scale = if offset != u16::MAX {
            let n = u64::from_le_bytes(swap[at..at + 8].try_into().unwrap());
            let d = u64::from_le_bytes(swap[at + 8..at + 16].try_into().unwrap());
            at += 16;
            Some((n, d))
        } else {
            None
        };
        hops.push((&swap[at..at + len], offset, scale));
        at += len;
    }
    let sy = GenericSy::parse(
        "9QReDA3ndt8FBQVJHGzM8h6NKojqyh2Ej8cqFn9dY6Hn"
            .parse()
            .unwrap(),
        include_bytes!("../../../tests/curator_data/sy.bin"),
    )
    .unwrap();
    assert_eq!(
        hops[1].0,
        sy.hop(key(30), TOKEN_PROGRAM_ID, 0, true, true)
            .unwrap()
            .data
    );
    assert_eq!(hops[1].1, 1);
    assert_eq!(hops[1].2.unwrap().1, INDEX_ONE);
    let m = Market::parse(
        "8JCVSxVrM1f5stBwNWko2D5Yt9p4wJQsvPVWFZBkzhmo"
            .parse()
            .unwrap(),
        include_bytes!("../../../tests/curator_data/market.bin"),
    )
    .unwrap();
    let table: Vec<_> = include_bytes!("../../../tests/curator_data/market_alt.bin")[56..]
        .as_chunks::<32>()
        .0
        .iter()
        .map(|b| Pubkey::new_from_array(*b))
        .collect();
    assert_eq!(
        hops[2].0,
        m.trade(key(30), 0, true, &table, true).unwrap().data
    );
    assert_eq!(hops[2].2, Some((1, 1)));
    let core = CoreVault::parse(
        m.vault,
        include_bytes!("../../../tests/curator_data/core.bin"),
    )
    .unwrap();
    let table: Vec<_> = include_bytes!("../../../tests/curator_data/core_alt.bin")[56..]
        .as_chunks::<32>()
        .0
        .iter()
        .map(|b| Pubkey::new_from_array(*b))
        .collect();
    assert_eq!(
        core.merge(key(30), 1, core.maturity + 1, &table)
            .unwrap()
            .accounts
            .len(),
        24
    );
}
#[test]
fn jupiter_build_checks_request_and_selector() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let q = quote(&manager(), true, 0);
    let wire = serde_json::json!({"inAmount":"1000","outAmount":"1000","otherAmountThreshold":"995","slippageBps":50,"swapMode":"ExactIn","routePlan":[{"swapInfo":{"inputMint":q.request.input_mint.to_string(),"outputMint":q.request.output_mint.to_string()}}],"swapInstruction":{"programId":jupiter::PROGRAM.to_string(),"accounts":q.hop.accounts.iter().map(|a|serde_json::json!({"pubkey":a.pubkey.to_string(),"isSigner":a.is_signer,"isWritable":a.is_writable})).collect::<Vec<_>>(),"data":STANDARD.encode(&q.hop.data)}});
    let response: jupiter::BuildResponse = serde_json::from_value(wire.clone()).unwrap();
    assert!(response.into_quote(q.request.clone()).is_ok());
    for (output, minimum, accepted) in [
        (201, 199, true),
        (201, 198, false),
        (999, 994, true),
        (999, 993, false),
    ] {
        let mut rounded = wire.clone();
        rounded["outAmount"] = output.to_string().into();
        rounded["otherAmountThreshold"] = minimum.to_string().into();
        assert_eq!(
            serde_json::from_value::<jupiter::BuildResponse>(rounded)
                .unwrap()
                .into_quote(q.request.clone())
                .is_ok(),
            accepted
        );
    }
    let mut wrong = wire.clone();
    wrong["inAmount"] = "999".into();
    assert!(
        serde_json::from_value::<jupiter::BuildResponse>(wrong)
            .unwrap()
            .into_quote(q.request.clone())
            .is_err()
    );
    let mut wrong = wire;
    wrong["swapInstruction"]["data"] = STANDARD.encode([0; 27]).into();
    assert!(
        serde_json::from_value::<jupiter::BuildResponse>(wrong)
            .unwrap()
            .into_quote(q.request)
            .is_err()
    );
    assert!(Intent::Sell(1).backend_request(key(1), 50).is_err());
    assert!(Intent::Redeem(1).backend_request(key(1), 50).is_err());
}
#[test]
fn jupiter_v2_amount_offsets_are_prefix_fields() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    for (selector, offset) in [
        (jupiter::EXACT_IN_SELECTORS[2], 8),
        (jupiter::EXACT_IN_SELECTORS[3], 9),
    ] {
        let q = quote(&manager(), false, 0);
        let mut data = selector.to_vec();
        if offset == 9 {
            data.push(7);
        }
        data.extend(1000u64.to_le_bytes());
        data.extend(1000u64.to_le_bytes());
        data.extend(50u16.to_le_bytes());
        data.extend(0u16.to_le_bytes());
        data.extend(0u16.to_le_bytes());
        data.extend(1u32.to_le_bytes());
        data.extend([0, 16, 39, 0, 1]);
        let wire = serde_json::json!({"inAmount":"1000","outAmount":"1000","otherAmountThreshold":"995","slippageBps":50,"swapMode":"ExactIn","routePlan":[{"swapInfo":{"inputMint":q.request.input_mint.to_string(),"outputMint":q.request.output_mint.to_string()}}],"swapInstruction":{"programId":jupiter::PROGRAM.to_string(),"accounts":q.hop.accounts.iter().map(|a|serde_json::json!({"pubkey":a.pubkey.to_string(),"isSigner":a.is_signer,"isWritable":a.is_writable})).collect::<Vec<_>>(),"data":STANDARD.encode(&data)}});
        let parsed = serde_json::from_value::<jupiter::BuildResponse>(wire)
            .unwrap()
            .into_quote(q.request)
            .unwrap();
        assert_eq!(
            parsed.hop(true).unwrap().amount_offset.unwrap().offset,
            offset as u16
        );
        assert_eq!(
            u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()),
            1000
        );
    }
}
#[test]
fn below_mid_sell_keeps_jupiter_quote_below_actual_redeem_output() {
    let actual_base = 995u64;
    let plan = run_with_quote(Intent::Sell(1000), 0, None, 100, |q| {
        q.request.amount = 995;
        q.output = 995;
        q.minimum = 990;
        let offset = q.hop.data.len() - 19;
        q.hop.data[offset..offset + 8].copy_from_slice(&995u64.to_le_bytes());
        q.hop.data[offset + 8..offset + 16].copy_from_slice(&995u64.to_le_bytes());
    })
    .unwrap();
    let Plan::Single(tx) = plan else {
        panic!("small sell should fit");
    };
    let data = &tx.instructions.last().unwrap().data;
    assert_eq!(u64::from_le_bytes(data[10..18].try_into().unwrap()), 990);
    let mut at = 27;
    let mut last: &[u8] = &[];
    let mut patch = 0;
    for _ in 0..3 {
        let len = u16::from_le_bytes(data[at + 1..at + 3].try_into().unwrap()) as usize;
        patch = u16::from_le_bytes(data[at + 3..at + 5].try_into().unwrap());
        at += 5;
        if patch != u16::MAX {
            at += 16;
        }
        last = &data[at..at + len];
        at += len;
    }
    let input = u64::from_le_bytes(
        last[usize::from(patch)..usize::from(patch) + 8]
            .try_into()
            .unwrap(),
    );
    let quoted = u64::from_le_bytes(
        last[usize::from(patch) + 8..usize::from(patch) + 16]
            .try_into()
            .unwrap(),
    );
    assert!(input <= actual_base);
    assert_eq!(quoted, input);
    assert!(actual_base * 10_000 >= quoted * 9950);
}

#[test]
fn mainnet_sell_and_redeem_account_goldens() {
    let owner = key(30);
    let market = Market::parse(
        "8JCVSxVrM1f5stBwNWko2D5Yt9p4wJQsvPVWFZBkzhmo"
            .parse()
            .unwrap(),
        include_bytes!("../../../tests/curator_data/market.bin"),
    )
    .unwrap();
    let sy = GenericSy::parse(
        "9QReDA3ndt8FBQVJHGzM8h6NKojqyh2Ej8cqFn9dY6Hn"
            .parse()
            .unwrap(),
        include_bytes!("../../../tests/curator_data/sy.bin"),
    )
    .unwrap();
    let core = CoreVault::parse(
        market.vault,
        include_bytes!("../../../tests/curator_data/core.bin"),
    )
    .unwrap();
    let ata =
        |mint| crate::chain::ix::derive_associated_token_address(&owner, &mint, &TOKEN_PROGRAM_ID);
    let table = |data: &[u8]| {
        data[56..]
            .as_chunks::<32>()
            .0
            .iter()
            .map(|b| Pubkey::new_from_array(*b))
            .collect::<Vec<_>>()
    };
    let expected = |text: &str| {
        text.lines()
            .map(|line| {
                let (key, flags) = line.split_once(' ').unwrap();
                let pubkey = match key {
                    "OWNER" => owner,
                    "SY_ATA" => ata(sy.mint),
                    "PT_ATA" => ata(core.pt),
                    "BASE_ATA" => ata(sy.base),
                    "YT_ATA" => ata(core.yt),
                    _ => key.parse().unwrap(),
                };
                AccountMeta {
                    pubkey,
                    is_signer: flags.contains('s'),
                    is_writable: flags.contains('w'),
                }
            })
            .collect::<Vec<_>>()
    };
    let sell = market
        .trade(
            owner,
            123,
            false,
            &table(include_bytes!("../../../tests/curator_data/market_alt.bin")),
            false,
        )
        .unwrap();
    let merge = core
        .merge(
            owner,
            123,
            core.maturity + 1,
            &table(include_bytes!("../../../tests/curator_data/core_alt.bin")),
        )
        .unwrap();
    for (hop, tag, golden, suffix) in [
        (
            &sell,
            3,
            include_str!("../../../tests/curator_data/trade_pt_accounts.txt"),
            vec![0, 0, 0],
        ),
        (
            &merge,
            5,
            include_str!("../../../tests/curator_data/merge_accounts.txt"),
            vec![],
        ),
    ] {
        assert_eq!(hop.accounts, expected(golden));
        assert_eq!(
            hop.data,
            [vec![tag], 123u64.to_le_bytes().to_vec(), suffix].concat()
        );
        assert_eq!((hop.source, hop.destination), (ata(core.pt), ata(sy.mint)));
        assert_eq!(hop.amount_offset, None);
        let redeem = sy.hop(owner, TOKEN_PROGRAM_ID, 0, false, true).unwrap();
        assert_eq!(
            redeem.accounts,
            expected(include_str!(
                "../../../tests/curator_data/sy_redeem_accounts.txt"
            ))
        );
        assert_eq!(redeem.data, [vec![2], 0u64.to_le_bytes().to_vec()].concat());
        assert_eq!(
            (redeem.source, redeem.destination),
            (hop.destination, ata(sy.base))
        );
        assert_eq!(
            redeem.amount_offset,
            Some(VaultSwapAmountOffset {
                offset: 1,
                num: 1,
                den: 1
            })
        );
    }
}

#[test]
fn jupiter_registry_has_all_eight_contract_selectors() {
    use sha2::{Digest, Sha256};
    let names = [
        "route",
        "shared_accounts_route",
        "exact_out_route",
        "shared_accounts_exact_out_route",
        "route_v2",
        "shared_accounts_route_v2",
        "exact_out_route_v2",
        "shared_accounts_exact_out_route_v2",
    ];
    for (name, selector) in names.into_iter().zip(jupiter::VENUE_SELECTORS) {
        assert_eq!(selector, Sha256::digest(format!("global:{name}"))[..8]);
    }
}

#[test]
fn main_slot_completion_respects_cycle_and_safety() {
    let mut account = manager().test_account(VaultStagingState {
        generation: 8,
        open: Some(VaultStagedSlot {
            mint: key(15),
            from_leg: VaultLeg::Main,
            amount: 1000,
            staged_at: 99,
        }),
    });
    account.active_expiry = 150;
    for phase in [
        VaultPhase::Idle,
        VaultPhase::Active,
        VaultPhase::Settling,
        VaultPhase::ProcessingCapital,
    ] {
        for safety in [
            VaultSafety::Normal,
            VaultSafety::Frozen,
            VaultSafety::EmergencyUnwind,
            VaultSafety::WithdrawOnly,
            VaultSafety::Closing,
        ] {
            account.phase = phase;
            account.safety = safety;
            for now in [149, 150, 151] {
                assert_eq!(
                    super::router::recovery_buy(&account, Intent::Buy(123), now, 200).unwrap(),
                    phase == VaultPhase::Active && safety == VaultSafety::Normal && now < 150
                );
            }
        }
    }
}
