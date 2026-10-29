use acta_maker_sdk::{PositionType, TakerSide, orders::*, spreads::*};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

const ORDER_VECTORS: &[u8] = include_bytes!("fixtures/order_preimage_vectors.json");

fn vectors() -> Vec<Value> {
    serde_json::from_slice(ORDER_VECTORS).expect("vendored order vector fixture")
}

fn key(args: &Value, name: &str) -> [u8; 32] {
    hex::decode(args[name].as_str().expect("hex fixture field"))
        .expect("valid fixture hex")
        .try_into()
        .expect("32-byte fixture key")
}

fn number(args: &Value, name: &str) -> u64 {
    args[name]
        .as_str()
        .expect("decimal string fixture field")
        .parse()
        .expect("u64 fixture field")
}

#[test]
fn vendored_order_vectors_match_the_backend_when_the_sibling_exists() {
    let canonical = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rust-backend/acta-ws-protocol/tests/fixtures/order_preimage_vectors.json");
    if canonical.is_file() {
        let canonical_bytes =
            std::fs::read(&canonical).expect("read canonical backend vector file");
        assert_eq!(
            Sha256::digest(&canonical_bytes),
            Sha256::digest(ORDER_VECTORS),
            "vendored vector checksum differs from {}",
            canonical.display()
        );
    }
}

#[test]
fn shared_type_tagged_preimages_hashes_and_mismatches_match_exactly() {
    let vectors = vectors();
    assert_eq!(vectors.len(), 17, "canonical fixture row count");
    assert_eq!(vectors.iter().filter(|v| v["valid"] == false).count(), 2);
    for vector in &vectors {
        let name = vector["name"].as_str().expect("name");
        let a = &vector["args"];
        let side: TakerSide = serde_json::from_value(a["side"].clone()).expect("side");
        assert_eq!(
            side.is_taker_buy(),
            a["is_taker_buy"].as_bool().expect("is_taker_buy")
        );
        let raw_type = u8::try_from(a["position_type"].as_u64().expect("type")).expect("type");
        let valid = vector["valid"].as_bool().expect("valid");
        let expected_preimage = vector["preimage_hex"].as_str().expect("preimage hex");
        let expected_order_id = vector["order_id"].as_str().expect("order id");
        assert!(expected_preimage.starts_with("41435441"), "{name}");
        let physical = |position_type| OrderPreimageArgs {
            chain_id: number(a, "chain_id"),
            program_id: key(a, "program_id"),
            side,
            position_type,
            market: key(a, "market"),
            strike: number(a, "strike"),
            quantity: number(a, "quantity"),
            gross_price: number(a, "gross_price"),
            valid_until: number(a, "valid_until"),
            maker: key(a, "maker"),
            taker: key(a, "taker"),
            nonce: number(a, "nonce"),
        };
        let Ok(spread_type) = SpreadType::try_from(raw_type) else {
            assert!(a.get("upper_strike").is_none(), "{name}");
            let position_type = match raw_type {
                0 => PositionType::CoveredCall,
                1 => PositionType::CashSecuredPut,
                other => panic!("unexpected type {other}"),
            };
            let preimage = build_order_preimage(&physical(position_type)).expect("physical");
            assert_eq!(hex::encode(preimage) == expected_preimage, valid, "{name}");
            let order_id = compute_order_id(&physical(position_type)).expect("physical id");
            assert_eq!(hex::encode(order_id) == expected_order_id, valid, "{name}");
            continue;
        };
        let args = SpreadOrderPreimageArgs {
            chain_id: number(a, "chain_id"),
            program_id: key(a, "program_id"),
            side,
            position_type: spread_type,
            market: key(a, "market"),
            lower_strike: number(a, "strike"),
            upper_strike: number(a, "upper_strike"),
            quantity: number(a, "quantity"),
            gross_price: number(a, "gross_price"),
            valid_until: number(a, "valid_until"),
            maker: key(a, "maker"),
            taker: key(a, "taker"),
            nonce: number(a, "nonce"),
        };
        let preimage = build_spread_order_preimage(&args).expect("spread layout");
        assert_eq!(hex::encode(preimage) == expected_preimage, valid, "{name}");
        let order_id = compute_spread_order_id(&args).expect("spread id");
        assert_eq!(hex::encode(order_id) == expected_order_id, valid, "{name}");
        let spread_position = match spread_type {
            SpreadType::CallSpread => PositionType::CallSpread,
            SpreadType::PutSpread => PositionType::PutSpread,
        };
        assert!(matches!(
            build_order_preimage(&physical(spread_position)),
            Err(OrderError::SpreadLayoutRequired)
        ));
        if !valid {
            continue;
        }
        if side == TakerSide::Buy {
            assert!(matches!(
                sign_spread_order(&args, &BytesSigner::from_secret([7; 32])),
                Err(SpreadError::TakerBuyUnsupported)
            ));
        } else {
            let signer = BytesSigner::from_secret([7; 32]);
            let (order_id, signature) = sign_spread_order(&args, &signer).expect("sell signs");
            verify_order_id_signature_bytes(&order_id, &signature, &signer.pubkey_bytes())
                .expect("signature verifies");
        }
    }
}

#[test]
fn buy_spread_preimages_remain_encodable_but_policy_refuses_signing() {
    let args = SpreadOrderPreimageArgs {
        chain_id: 0,
        program_id: [4; 32],
        side: TakerSide::Buy,
        position_type: SpreadType::CallSpread,
        market: [1; 32],
        lower_strike: 100_000_000_000,
        upper_strike: 120_000_000_000,
        quantity: 1_000_000,
        gross_price: 1_000_000_000,
        valid_until: 123,
        maker: [2; 32],
        taker: [3; 32],
        nonce: 999,
    };
    let preimage = build_spread_order_preimage(&args).expect("raw V2 preimage supports buy");
    assert_eq!(preimage[44], 1);
    assert!(compute_spread_order_id(&args).is_ok());
    assert!(matches!(
        sign_spread_order(&args, &BytesSigner::from_secret([7; 32])),
        Err(SpreadError::TakerBuyUnsupported)
    ));
}

fn terms(kind: SpreadType) -> SpreadTerms {
    SpreadTerms {
        kind,
        lower_strike: 100_000_000_000,
        upper_strike: 120_000_000_000,
        quantity: 1_000_001,
        quote_decimals: 6,
        underlying_decimals: 9,
    }
}

#[test]
fn cash_math_rounding_donations_and_fee_composition() {
    let call = terms(SpreadType::CallSpread);
    let put = terms(SpreadType::PutSpread);
    assert_eq!(call.collateral().unwrap(), 20_001);
    for price in [
        0,
        100_000_000_000,
        110_000_000_000,
        120_000_000_000,
        u64::MAX,
    ] {
        for t in [call, put] {
            let split = t.settle(price, 20_010).unwrap();
            assert_eq!(split.maker_payoff + split.taker_return, 20_010);
            assert!(split.maker_payoff <= 20_001);
        }
    }
    assert_eq!(call.payoff(100_000_000_001).unwrap(), 1);
    assert_eq!(
        call.payoff(120_000_000_000).unwrap(),
        call.collateral().unwrap()
    );
    assert_eq!(
        put.payoff(100_000_000_000).unwrap(),
        put.collateral().unwrap()
    );
    assert!(call.settle(110_000_000_000, 20_000).is_err());
    let fee = call.premium(1_000_000_000, 1, 100).unwrap();
    assert_eq!(fee.gross_premium, 1000);
    assert_eq!(fee.protocol_fee, 1); // floor(G*bps/10000) would be zero.
    assert_eq!(fee.net_premium + fee.protocol_fee, fee.gross_premium);
    assert!(call.premium(20_000_000_000, 100, 100).is_err());
    assert!(validate_strikes(SpreadType::CallSpread, 0, 10).is_err());
    assert!(validate_strikes(SpreadType::CallSpread, 10, 10).is_err());
    assert!(validate_strikes(SpreadType::CallSpread, 11, 10).is_err());
    assert_eq!(
        validate_strikes(SpreadType::CallSpread, 10_000_001, 20_000_003).unwrap(),
        10_000_002
    );
    assert_eq!(
        validate_strikes(SpreadType::CallSpread, 1, u64::MAX).unwrap(),
        u64::MAX - 1
    );
    assert!(quote_amount(u64::MAX, u64::MAX, 18, 0, true).is_err());
    assert!(quote_amount(1, 1, 19, 0, false).is_err());
    assert_eq!(quote_amount(u64::MAX, 1, 9, 0, true).unwrap(), u64::MAX);
    let historical = SpreadTerms {
        lower_strike: 1,
        upper_strike: 2,
        ..call
    };
    assert!(historical.payoff(2).is_ok());
    assert!(historical.premium(1, 0, 0).is_err());
}

#[test]
fn pricing_uses_one_context_and_put_orientation() {
    let context = SpreadPricingContext {
        time_to_expiry_years: 0.5,
        volatility: 0.7,
    };
    for kind in [SpreadType::CallSpread, SpreadType::PutSpread] {
        let result = price_spread(kind, 100.0, 120.0, context, |put, strike, received| {
            assert_eq!(received, context);
            if put { strike - 90.0 } else { 130.0 - strike }
        })
        .unwrap();
        assert_eq!(result, 20.0);
    }
    assert!(
        price_spread(SpreadType::CallSpread, 100.0, 120.0, context, |_, _, _| {
            f64::NAN
        })
        .is_err()
    );
}

proptest::proptest! {
    #[test]
    fn payoff_conserves_atoms(qty in 1u64..1_000_000_000, spot in 0u64..200_000_000_000) {
        for kind in [SpreadType::CallSpread, SpreadType::PutSpread] {
            let t = SpreadTerms { quantity: qty, ..terms(kind) };
            let collateral = t.collateral().unwrap();
            let split = t.settle(spot, collateral + 7).unwrap();
            proptest::prop_assert!(split.maker_payoff <= collateral);
            proptest::prop_assert_eq!(split.maker_payoff + split.taker_return, collateral + 7);
        }
    }
}

#[cfg(feature = "chain")]
#[test]
fn chain_layout_events_open_and_settle() {
    use acta_maker_sdk::{
        PositionType,
        chain::{spreads::*, *},
        types::*,
    };
    use solana_sdk::{account::Account, pubkey::Pubkey};
    let program = Pubkey::new_from_array([4; 32]);
    let maker = Pubkey::new_from_array([2; 32]);
    let taker = Pubkey::new_from_array([3; 32]);
    let token = solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
    let mut market = MarketInfo {
        expiry_ts: 200,
        is_put: true,
        is_finalized: false,
        settlement_price: 110_000_000_000,
        underlying_mint: Pubkey::new_from_array([10; 32]),
        quote_mint: Pubkey::new_from_array([11; 32]),
        underlying_token_program_id: token,
        quote_token_program_id: token,
        underlying_decimals: 9.into(),
        quote_decimals: 6.into(),
    };
    let market_pda = Pubkey::find_program_address(
        &[
            b"market",
            market.underlying_mint.as_ref(),
            market.quote_mint.as_ref(),
            &200u64.to_le_bytes(),
            &[1],
        ],
        &program,
    )
    .0;
    let signer = BytesSigner::from_secret([7; 32]);
    let args = SpreadOrderPreimageArgs {
        chain_id: 0,
        program_id: program.to_bytes(),
        side: TakerSide::Sell,
        position_type: SpreadType::PutSpread,
        market: market_pda.to_bytes(),
        lower_strike: 100_000_000_000,
        upper_strike: 120_000_000_000,
        quantity: 1_000_001,
        gross_price: 1_000_000_000,
        valid_until: 123,
        maker: maker.to_bytes(),
        taker: taker.to_bytes(),
        nonce: 999,
    };
    let (id, sig) = sign_spread_order(&args, &signer).unwrap();
    let mut buy = args.clone();
    buy.side = TakerSide::Buy;
    assert!(matches!(
        build_open_spread_ixs(&OpenSpreadIxArgs {
            order: &buy,
            market: &market,
            keeper: maker,
            quote_signer: signer.pubkey_bytes(),
            signature: sig,
        }),
        Err(ChainError::TakerBuyUnsupported)
    ));
    let ixs = build_open_spread_ixs(&OpenSpreadIxArgs {
        order: &args,
        market: &market,
        keeper: maker,
        quote_signer: signer.pubkey_bytes(),
        signature: sig,
    })
    .unwrap();
    let open = &ixs[1];
    assert_eq!(open.data.len(), 82);
    assert_eq!(open.data[0], 77);
    assert_eq!(open.accounts.len(), 16);
    assert_eq!(open.accounts[2], open.accounts[3]);
    assert_eq!(&open.data[42..74], &id);
    assert_eq!(&open.data[74..], &args.upper_strike.to_le_bytes());
    assert_eq!(&ixs[0].data[112..], &id);
    let mut data = vec![0; 424];
    data[0] = 1;
    data[1] = 2;
    data[3] = 3;
    data[4] = 1;
    data[8..40].copy_from_slice(taker.as_ref());
    data[40..72].copy_from_slice(maker.as_ref());
    data[72..104].copy_from_slice(market_pda.as_ref());
    data[104..112].copy_from_slice(&args.lower_strike.to_le_bytes());
    data[112..120].copy_from_slice(&args.quantity.to_le_bytes());
    data[128..160].copy_from_slice(&id);
    data[168..176].copy_from_slice(&args.upper_strike.to_le_bytes());
    let mut account = Account {
        lamports: 1,
        data,
        owner: program,
        executable: false,
        rent_epoch: 0,
    };
    let mut position = PositionInfo::from_account(&account, &program).unwrap();
    assert_eq!(
        position.terms,
        OptionTerms::PutSpread {
            lower_strike: Strike::new(args.lower_strike),
            upper_strike: Strike::new(args.upper_strike),
        }
    );
    assert!(build_cash_settle_ix(program, maker, &position, &market).is_err());
    market.is_finalized = true;
    let settle = build_cash_settle_ix(program, maker, &position, &market).unwrap();
    assert_eq!(settle.data, [10, 0]);
    assert_eq!(settle.accounts.len(), 9);
    assert!(!settle.accounts[4].is_writable);
    assert_eq!(settle.accounts[3].pubkey, open.accounts[8].pubkey);
    let mut buy_position = position.clone();
    buy_position.side = TakerSide::Buy;
    assert!(matches!(
        build_cash_settle_ix(program, maker, &buy_position, &market),
        Err(ChainError::TakerBuyUnsupported)
    ));
    // One owner on both sides settles both amounts into one shared quote account.
    let mut same_owner = position.clone();
    same_owner.taker_owner = same_owner.maker_owner;
    let shared = build_cash_settle_ix(program, maker, &same_owner, &market).unwrap();
    assert_eq!(shared.accounts[5].pubkey, shared.accounts[6].pubkey);
    let custom = CashSettleDestinations {
        taker: Pubkey::new_unique(),
        maker: Pubkey::new_unique(),
    };
    let custom_ix =
        build_cash_settle_ix_with_destinations(program, maker, &same_owner, &market, custom)
            .unwrap();
    assert_eq!(custom_ix.accounts[5].pubkey, custom.taker);
    assert_eq!(custom_ix.accounts[6].pubkey, custom.maker);
    assert!(
        build_cash_settle_ix_with_destinations(
            program,
            maker,
            &position,
            &market,
            CashSettleDestinations {
                taker: custom.maker,
                maker: custom.maker
            }
        )
        .is_err()
    );
    position.status = PositionStatus::Funded;
    assert!(build_cash_settle_ix(program, maker, &position, &market).is_err());
    position.status = PositionStatus::Open;
    // A vault taker settles into its canonical quote ATA, followed by the vault tail.
    position.flags = 1;
    let vault_settle = build_cash_settle_ix(program, maker, &position, &market).unwrap();
    assert_eq!(vault_settle.accounts.len(), 12);
    assert_eq!(vault_settle.accounts[11].pubkey, position.taker_owner);
    assert!(vault_settle.accounts[11].is_writable);
    assert!(
        build_cash_settle_ix_with_destinations(program, maker, &position, &market, custom).is_err()
    );
    position.flags = 4;
    assert!(build_cash_settle_ix(program, maker, &position, &market).is_err());
    assert!(
        ix::build_fund_position_ixs(
            &program,
            &ix::FundPositionIxArgs {
                maker_owner: maker,
                position_pda: open.accounts[7].pubkey,
                market_pda,
                position_type: 3,
                underlying_mint: market.underlying_mint,
                quote_mint: market.quote_mint,
                underlying_token_program_id: token,
                quote_token_program_id: token,
                create_atas: false
            },
            maker
        )
        .is_err()
    );
    account.data[3] = 0;
    assert_eq!(
        PositionInfo::from_account(&account, &program)
            .unwrap()
            .terms,
        OptionTerms::CoveredCall {
            strike: Strike::new(args.lower_strike),
        }
    );
    assert!(
        build_order_preimage(&OrderPreimageArgs {
            chain_id: 0,
            program_id: [4; 32],
            side: TakerSide::Sell,
            position_type: PositionType::CallSpread,
            market: [1; 32],
            strike: args.lower_strike,
            quantity: args.quantity,
            gross_price: args.gross_price,
            valid_until: 123,
            maker: [2; 32],
            taker: [3; 32],
            nonce: 999
        })
        .is_err()
    );
    let mut event = vec![0; 223];
    event[..6].copy_from_slice(b"ACTA\x01\x4f");
    event[134] = 3;
    event[215..].copy_from_slice(&120u64.to_le_bytes());
    assert!(matches!(
        events::decode_spread_event(&event).unwrap(),
        Some(events::SpreadEvent::Open(events::OpenSpreadEvent {
            upper_strike: 120,
            ..
        }))
    ));
    event.push(0);
    assert!(events::decode_spread_event(&event).is_err());
    // settler, taker, maker owner, position, market; status, is_otm, payoff, return.
    let mut event = vec![0; 184];
    event[..6].copy_from_slice(b"ACTA\x01\x50");
    event[38..70].copy_from_slice(&[7; 32]);
    event[166] = 4;
    event[168..176].copy_from_slice(&9u64.to_le_bytes());
    event[176..184].copy_from_slice(&12u64.to_le_bytes());
    let Some(events::SpreadEvent::Settle(settle)) = events::decode_spread_event(&event).unwrap()
    else {
        panic!("settle event");
    };
    assert_eq!(
        (settle.taker, settle.maker_payoff, settle.taker_return),
        ([7; 32], 9, 12)
    );
}
