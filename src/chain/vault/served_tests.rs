use std::str::FromStr;

use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{AddressLookupTableAccount, Message, VersionedMessage, v0},
    pubkey::Pubkey,
};
use uuid::Uuid;

use super::*;
use crate::chain::vault::{MAX_VAULT_SWAP_ROUTE_ACCOUNTS, VaultManager, VaultManagerArgs};
use crate::types::OrderId;
use crate::ws::types::{SponsoredTxToSignData, SwapQuoteData, SwapQuoteResultData};

const FIXTURE: &str = include_str!("../../../tests/fixtures/vault_served_tx.json");
const BOUNDS: ComputeBudgetBounds = ComputeBudgetBounds {
    max_unit_limit: 400_000,
    max_unit_price_micro_lamports: 1_000,
};

fn cases() -> Vec<Value> {
    serde_json::from_str::<Vec<Value>>(FIXTURE).unwrap()
}

fn key(value: &Value) -> Pubkey {
    Pubkey::from_str(value.as_str().unwrap()).unwrap()
}

fn num(value: &Value) -> u64 {
    value.as_u64().unwrap()
}

fn manager(case: &Value, field: &str) -> VaultManager {
    let vault = &case[field];
    VaultManager::new(VaultManagerArgs {
        program_id: key(&case["program_id"]),
        vault_id: hex::decode(vault["vault_id"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
        actor: key(&vault["actor"]),
        delegate_authority: key(&vault["delegate_authority"]),
        main_mint: key(&vault["main_mint"]),
        second_mint: key(&vault["second_mint"]),
        share_mint: key(&vault["share_mint"]),
        main_token_program: key(&vault["main_token_program"]),
        second_token_program: key(&vault["second_token_program"]),
    })
    .unwrap()
}

fn terms(case: &Value) -> VaultTradeTerms {
    let p = &case["position"];
    VaultTradeTerms {
        underlying_mint: key(&p["underlying_mint"]),
        quote_mint: key(&p["quote_mint"]),
        expiry_ts: num(&p["expiry_ts"]),
        is_put: p["is_put"].as_bool().unwrap(),
        strike: num(&p["strike"]),
        quantity: num(&p["quantity"]),
        price: num(&p["price"]),
        valid_until: num(&p["valid_until"]),
        nonce: num(&p["nonce"]),
    }
}

fn table(case: &Value) -> AddressLookupTableAccount {
    let table = &case["served"]["lookup_table"];
    AddressLookupTableAccount {
        key: key(&table["key"]),
        addresses: table["addresses"]
            .as_array()
            .unwrap()
            .iter()
            .map(key)
            .collect(),
    }
}

fn served_tx(case: &Value) -> String {
    case["served"]["tx_base64"].as_str().unwrap().to_owned()
}

fn swap_request(case: &Value) -> SwapQuoteData {
    let swap = &case["swap"];
    SwapQuoteData {
        request_id: Uuid::from_str(swap["request_id"].as_str().unwrap()).unwrap(),
        vault_pda: manager(case, "swapper").addresses.vault.to_string(),
        mode: VaultSwapMode::Rebalance,
        direction: serde_json::from_value(swap["direction"].clone()).unwrap(),
        amount: num(&swap["amount"]).to_string(),
        slippage_bps: u16::try_from(num(&swap["slippage_bps"])).unwrap(),
    }
}

fn swap_quote(case: &Value, tx_base64: String) -> SwapQuoteResultData {
    let swap = &case["swap"];
    SwapQuoteResultData {
        request_id: Uuid::from_str(swap["request_id"].as_str().unwrap()).unwrap(),
        execution_id: OrderId::new([4; 32]),
        mode: VaultSwapMode::Rebalance,
        tx_base64,
        last_valid_block_height: 1,
        in_amount: num(&swap["amount"]).to_string(),
        quoted_out_amount: num(&swap["quoted_out_amount"]).to_string(),
        minimum_out_amount: num(&swap["minimum_out_amount"]).to_string(),
        valid_until: num(&swap["valid_until"]).to_string(),
        main_price_update: swap["main_price_update"].as_str().unwrap().to_owned(),
        second_price_update: swap["second_price_update"].as_str().unwrap().to_owned(),
    }
}

fn price_feeds(case: &Value) -> PairPriceFeeds<'static> {
    let feed = |name: &str| {
        let mut id = [0; 32];
        hex::decode_to_slice(case["swap"][name].as_str().unwrap(), &mut id).unwrap();
        id
    };
    assert_eq!(case["swap"]["shards"], serde_json::json!([0, 1]));
    PairPriceFeeds {
        main: feed("main_feed"),
        second: feed("second_feed"),
        shards: &[0, 1],
    }
}

fn verify_swap(
    case: &Value,
    request: &SwapQuoteData,
    quote: &SwapQuoteResultData,
    tables: &[AddressLookupTableAccount],
) -> Result<(), VaultIntentError> {
    let floor = num(&case["swap"]["minimum_out_amount"]);
    verify_swap_with_floor(case, request, quote, tables, floor)
}

fn verify_swap_with_floor(
    case: &Value,
    request: &SwapQuoteData,
    quote: &SwapQuoteResultData,
    tables: &[AddressLookupTableAccount],
    min_output_floor: u64,
) -> Result<(), VaultIntentError> {
    let vault = manager(case, "swapper");
    verify_vault_swap(
        &VaultSwapIntent {
            vault: &vault,
            request,
            quote,
            price_feeds: price_feeds(case),
            min_output_floor,
            router: crate::chain::vault::JUPITER_V6_PROGRAM_ID,
        },
        tables,
        BOUNDS,
    )
    .map(|_| ())
}

fn verify(
    case: &Value,
    tx_base64: &str,
    tables: &[AddressLookupTableAccount],
) -> Result<(), VaultIntentError> {
    let terms = terms_or_default(case);
    let order_id = match case["position"]["order_id"].as_str() {
        Some(hex) => OrderId::from_hex(hex).unwrap(),
        None => OrderId::new([0; 32]),
    };
    let to_sign = &SponsoredTxToSignData {
        order_id,
        tx_base64: tx_base64.to_owned(),
        last_valid_block_height: 1,
        signature_deadline: std::time::UNIX_EPOCH,
    };
    match case["kind"].as_str().unwrap() {
        "vault_taker" => {
            let writer = manager(case, "writer");
            verify_vault_open(
                to_sign,
                &VaultOpenIntent::Sale {
                    writer: &writer,
                    maker_owner: key(&case["maker_owner"]),
                    maker_quote_signer: key(&case["position"]["quote_signer"]),
                    terms,
                },
                tables,
                BOUNDS,
            )
            .map(|_| ())
        }
        "vault_to_vault" => {
            let (writer, holder) = (manager(case, "writer"), manager(case, "holder"));
            verify_vault_open(
                to_sign,
                &VaultOpenIntent::BetweenVaults {
                    writer: &writer,
                    holder: &holder,
                    terms,
                },
                tables,
                BOUNDS,
            )
            .map(|_| ())
        }
        "vault_maker" => {
            let holder = manager(case, "maker");
            verify_vault_open(
                to_sign,
                &VaultOpenIntent::Fill {
                    holder: &holder,
                    taker: key(&case["taker"]),
                    terms,
                },
                tables,
                BOUNDS,
            )
            .map(|_| ())
        }
        "swap" => verify_swap(
            case,
            &swap_request(case),
            &swap_quote(case, tx_base64.to_owned()),
            tables,
        ),
        other => panic!("unknown case {other}"),
    }
}

fn terms_or_default(case: &Value) -> VaultTradeTerms {
    if case["position"].is_object() {
        terms(case)
    } else {
        VaultTradeTerms {
            underlying_mint: Pubkey::default(),
            quote_mint: Pubkey::default(),
            expiry_ts: 0,
            is_put: false,
            strike: 0,
            quantity: 0,
            price: 0,
            valid_until: 0,
            nonce: 0,
        }
    }
}

fn decoded(case: &Value) -> (Pubkey, Vec<Instruction>) {
    let served = ServedMessage::decode(&served_tx(case), &[table(case)]).unwrap();
    (served.keys[0].0, served.instructions)
}

fn encode(message: VersionedMessage) -> String {
    let signers = usize::from(message.header().num_required_signatures);
    let mut bytes = vec![u8::try_from(signers).unwrap()];
    bytes.extend(std::iter::repeat_n(0u8, signers * 64));
    bytes.extend(message.serialize());
    STANDARD.encode(bytes)
}

fn compile(payer: Pubkey, ixs: &[Instruction], tables: &[AddressLookupTableAccount]) -> String {
    encode(VersionedMessage::V0(
        v0::Message::try_compile(&payer, ixs, tables, Hash::new_from_array([9; 32])).unwrap(),
    ))
}

fn route_start(case: &Value, index: usize, ix: &Instruction) -> (usize, usize) {
    if case["kind"] == "swap" && index == 2 {
        (18, 28)
    } else {
        (ix.accounts.len(), ix.data.len())
    }
}

#[test]
fn every_served_fixture_verifies() {
    let cases = cases();
    assert_eq!(cases.len(), 16);
    for case in &cases {
        verify(case, &served_tx(case), &[table(case)])
            .unwrap_or_else(|error| panic!("{} rejected: {error}", case["kind"]));
    }
}

#[test]
fn recompiled_and_legacy_messages_verify() {
    for case in &cases() {
        let (payer, ixs) = decoded(case);
        let table = table(case);
        assert_eq!(
            compile(payer, &ixs, std::slice::from_ref(&table)),
            served_tx(case)
        );
        let legacy = encode(VersionedMessage::Legacy(Message::new_with_blockhash(
            &ixs,
            Some(&payer),
            &Hash::new_from_array([3; 32]),
        )));
        verify(case, &legacy, &[]).unwrap();
    }
}

#[test]
fn any_account_flag_or_data_change_is_rejected() {
    for case in &cases() {
        let (payer, ixs) = decoded(case);
        let tables = [table(case)];
        let baseline = compile(payer, &ixs, &tables);
        let rejects = |mutated: &[Instruction], what: String| {
            let tx = compile(payer, mutated, &tables);
            if tx != baseline {
                assert!(
                    verify(case, &tx, &tables).is_err(),
                    "{} accepted {what}",
                    case["kind"]
                );
            }
        };
        for (index, ix) in ixs.iter().enumerate() {
            let (route_accounts, route_data) = route_start(case, index, ix);
            for position in 0..route_accounts {
                let mut replaced = ixs.clone();
                replaced[index].accounts[position].pubkey = Pubkey::new_unique();
                rejects(&replaced, format!("ix {index} account {position} replaced"));
                if replaced[index].accounts[position].pubkey != payer {
                    let mut writable = ixs.clone();
                    let meta = &mut writable[index].accounts[position];
                    meta.is_writable = !meta.is_writable;
                    rejects(&writable, format!("ix {index} account {position} writable"));
                    let mut signer = ixs.clone();
                    signer[index].accounts[position].is_signer = true;
                    rejects(&signer, format!("ix {index} account {position} signer"));
                }
            }
            let mut program = ixs.clone();
            program[index].program_id = Pubkey::new_unique();
            rejects(&program, format!("ix {index} program"));
            if index < 2 {
                continue;
            }
            for byte in 0..route_data {
                let mut data = ixs.clone();
                data[index].data[byte] ^= 1;
                rejects(&data, format!("ix {index} data byte {byte}"));
            }
            let mut longer = ixs.clone();
            longer[index].data.push(0);
            rejects(&longer, format!("ix {index} trailing data"));
        }
    }
}

#[test]
fn extra_missing_or_reordered_instructions_are_rejected() {
    for case in &cases() {
        let (payer, ixs) = decoded(case);
        let tables = [table(case)];
        let transfer = Instruction {
            program_id: SYSTEM_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(Pubkey::new_unique(), false),
            ],
            data: [2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0].to_vec(),
        };
        for index in 0..=ixs.len() {
            let mut extra = ixs.clone();
            extra.insert(index, transfer.clone());
            assert!(verify(case, &compile(payer, &extra, &tables), &tables).is_err());
        }
        for index in 0..ixs.len() {
            let mut missing = ixs.clone();
            missing.remove(index);
            assert!(verify(case, &compile(payer, &missing, &tables), &tables).is_err());
        }
        let mut swapped = ixs.clone();
        swapped.swap(0, 1);
        assert!(verify(case, &compile(payer, &swapped, &tables), &tables).is_err());
        if let Some(position) = ixs
            .iter()
            .position(|ix| ix.program_id == ASSOCIATED_TOKEN_PROGRAM_ID)
        {
            let mut rent = ixs.clone();
            rent[position].accounts.push(AccountMeta::new_readonly(
                solana_sdk::pubkey!("SysvarRent111111111111111111111111111111111"),
                false,
            ));
            assert!(matches!(
                verify(case, &compile(payer, &rent, &tables), &tables),
                Err(VaultIntentError::AccountCount { .. })
            ));
        }
    }
}

#[test]
fn wrong_payer_budget_or_table_is_rejected() {
    for case in &cases() {
        let (payer, ixs) = decoded(case);
        let tables = [table(case)];
        assert!(matches!(
            verify(case, &compile(Pubkey::new_unique(), &ixs, &tables), &tables),
            Err(VaultIntentError::FeePayer { .. })
        ));
        assert!(matches!(
            verify(case, &served_tx(case), &[]),
            Err(VaultIntentError::MissingLookupTable(_))
        ));
        let mut limit = ixs.clone();
        limit[0].data[1..].copy_from_slice(&400_001u32.to_le_bytes());
        assert!(matches!(
            verify(case, &compile(payer, &limit, &tables), &tables),
            Err(VaultIntentError::ComputeUnitLimit { limit: 400_001, .. })
        ));
        let mut price = ixs.clone();
        price[1].data[1..].copy_from_slice(&1_001u64.to_le_bytes());
        assert!(matches!(
            verify(case, &compile(payer, &price, &tables), &tables),
            Err(VaultIntentError::ComputeUnitPrice { price: 1_001, .. })
        ));
        let mut lower = ixs.clone();
        lower[0].data[1..].copy_from_slice(&200_000u32.to_le_bytes());
        verify(case, &compile(payer, &lower, &tables), &tables).unwrap();
    }
}

#[test]
fn open_terms_and_counterparties_are_bound() {
    for case in cases().iter().filter(|case| case["position"].is_object()) {
        let tx = served_tx(case);
        let tables = [table(case)];
        let mutations: [fn(&mut VaultTradeTerms); 7] = [
            |terms: &mut VaultTradeTerms| terms.strike += 1,
            |terms: &mut VaultTradeTerms| terms.quantity += 1,
            |terms: &mut VaultTradeTerms| terms.price += 1,
            |terms: &mut VaultTradeTerms| terms.valid_until -= 1,
            |terms: &mut VaultTradeTerms| terms.nonce += 1,
            |terms: &mut VaultTradeTerms| terms.expiry_ts += 1,
            |terms: &mut VaultTradeTerms| terms.is_put = !terms.is_put,
        ];
        for mutate in mutations {
            let mut changed = case.clone();
            let mut terms = terms(case);
            mutate(&mut terms);
            let p = &mut changed["position"];
            p["strike"] = terms.strike.into();
            p["quantity"] = terms.quantity.into();
            p["price"] = terms.price.into();
            p["valid_until"] = terms.valid_until.into();
            p["nonce"] = terms.nonce.into();
            p["expiry_ts"] = terms.expiry_ts.into();
            p["is_put"] = terms.is_put.into();
            assert!(verify(&changed, &tx, &tables).is_err());
        }
        let mut stranger = case.clone();
        match case["kind"].as_str().unwrap() {
            "vault_taker" => stranger["maker_owner"] = Pubkey::new_unique().to_string().into(),
            "vault_maker" => stranger["taker"] = Pubkey::new_unique().to_string().into(),
            _ => stranger["holder"]["vault_id"] = hex::encode([0x77; 32]).into(),
        }
        assert!(verify(&stranger, &tx, &tables).is_err());
        let mut signer = case.clone();
        signer["position"]["quote_signer"] = Pubkey::new_unique().to_string().into();
        signer["holder"]["delegate_authority"] = signer["position"]["quote_signer"].clone();
        signer["maker"]["delegate_authority"] = signer["position"]["quote_signer"].clone();
        assert!(verify(&signer, &tx, &tables).is_err());
    }
}

#[test]
fn quote_signature_must_verify() {
    for case in cases().iter().filter(|case| case["position"].is_object()) {
        let (payer, mut ixs) = decoded(case);
        let tables = [table(case)];
        let ed = ixs.len() - 2;
        ixs[ed].data[60] ^= 1;
        assert_eq!(
            verify(case, &compile(payer, &ixs, &tables), &tables),
            Err(VaultIntentError::QuoteSignature(
                "signature does not verify"
            ))
        );
    }
}

#[test]
fn swap_terms_follow_the_request() {
    let tables_of = |case: &Value| [table(case)];
    for case in cases().iter().filter(|case| case["kind"] == "swap") {
        let tables = tables_of(case);
        let request = swap_request(case);
        let quote = swap_quote(case, served_tx(case));
        verify_swap(case, &request, &quote, &tables).unwrap();
        let vault = manager(case, "swapper");
        assert!(
            verify_vault_swap(
                &VaultSwapIntent {
                    vault: &vault,
                    request: &request,
                    quote: &quote,
                    price_feeds: price_feeds(case),
                    min_output_floor: 1,
                    router: crate::chain::vault::DEVNET_SWAP_ROUTER_ID,
                },
                &tables,
                BOUNDS,
            )
            .is_err(),
            "a route served for another router must not verify"
        );

        let mut loose = quote.clone();
        loose.minimum_out_amount = "1".into();
        assert!(matches!(
            verify_swap(case, &request, &loose, &tables),
            Err(VaultIntentError::Terms(_))
        ));
        let mut bigger = request.clone();
        bigger.amount = "1000001".into();
        assert!(matches!(
            verify_swap(case, &bigger, &quote, &tables),
            Err(VaultIntentError::Terms(_))
        ));
        let mut other = request.clone();
        other.request_id = Uuid::new_v4();
        assert!(verify_swap(case, &other, &quote, &tables).is_err());
        let mut recovery = request.clone();
        recovery.mode = VaultSwapMode::Recovery;
        recovery.direction = VaultSwapDirection::MainToSecond;
        let mut recovery_quote = quote.clone();
        recovery_quote.mode = VaultSwapMode::Recovery;
        assert!(matches!(
            verify_swap(case, &recovery, &recovery_quote, &tables),
            Err(VaultIntentError::Terms(_))
        ));
        let mut reversed = request.clone();
        reversed.direction = match request.direction {
            VaultSwapDirection::MainToSecond => VaultSwapDirection::SecondToMain,
            _ => VaultSwapDirection::MainToSecond,
        };
        assert!(matches!(
            verify_swap(case, &reversed, &quote, &tables),
            Err(VaultIntentError::Data { .. })
        ));
        for (field, value) in [("min", "1991"), ("valid", "1000101")] {
            let mut changed = quote.clone();
            match field {
                "min" => changed.minimum_out_amount = value.into(),
                _ => changed.valid_until = value.into(),
            }
            assert!(matches!(
                verify_swap(case, &request, &changed, &tables),
                Err(VaultIntentError::Data { .. })
            ));
        }
        let unconfigured = crate::chain::vault::pyth_price_feed_account(&price_feeds(case).main, 2);
        let mut other_shard = quote.clone();
        other_shard.main_price_update = unconfigured.to_string();
        assert_eq!(
            verify_swap(case, &request, &other_shard, &tables),
            Err(VaultIntentError::PriceUpdate("main"))
        );
        let mut crossed = quote.clone();
        crossed.second_price_update = quote.main_price_update.clone();
        assert_eq!(
            verify_swap(case, &request, &crossed, &tables),
            Err(VaultIntentError::PriceUpdate("second"))
        );
    }
}

#[test]
fn slippage_minimum_is_floored_like_the_router() {
    for case in cases().iter().filter(|case| case["kind"] == "swap") {
        let tables = [table(case)];
        let request = swap_request(case);
        let mut quote = swap_quote(case, served_tx(case));
        quote.quoted_out_amount = "2001".into();
        verify_swap(case, &request, &quote, &tables).unwrap();

        let mut below = quote.clone();
        below.minimum_out_amount = "1989".into();
        assert_eq!(
            verify_swap(case, &request, &below, &tables),
            Err(VaultIntentError::Terms(
                "minimum output exceeds the requested slippage"
            ))
        );
        assert_eq!(
            verify_swap_with_floor(case, &request, &quote, &tables, 1991),
            Err(VaultIntentError::Terms(
                "minimum output is below the operator's floor"
            ))
        );
    }
}

#[test]
fn served_minimum_below_the_operator_floor_is_rejected() {
    for case in cases().iter().filter(|case| case["kind"] == "swap") {
        let (payer, mut ixs) = decoded(case);
        let tables = [table(case)];
        let request = swap_request(case);
        let kind = VaultSwapKind::Rebalance {
            second_to_main: request.direction == VaultSwapDirection::SecondToMain,
        };
        ixs[2] = manager(case, "swapper")
            .swap(
                kind,
                &VaultSwapRoute {
                    router: crate::chain::vault::JUPITER_V6_PROGRAM_ID,
                    main_price_update: key(&case["swap"]["main_price_update"]),
                    second_price_update: key(&case["swap"]["second_price_update"]),
                    amount_in: num(&case["swap"]["amount"]),
                    minimum_out: 1,
                    valid_until: num(&case["swap"]["valid_until"]),
                    route_accounts: ixs[2].accounts[18..].to_vec(),
                    route_data: ixs[2].data[28..].to_vec(),
                },
            )
            .unwrap();
        let mut quote = swap_quote(case, compile(payer, &ixs, &tables));
        quote.quoted_out_amount = "1".into();
        quote.minimum_out_amount = "1".into();
        verify_swap_with_floor(case, &request, &quote, &tables, 0).unwrap();
        assert_eq!(
            verify_swap(case, &request, &quote, &tables),
            Err(VaultIntentError::Terms(
                "minimum output is below the operator's floor"
            ))
        );
    }
}

#[test]
fn swap_route_is_confined_and_bounded() {
    for case in cases().iter().filter(|case| case["kind"] == "swap") {
        let (payer, ixs) = decoded(case);
        let tables = [table(case)];
        let vault = manager(case, "swapper");
        let check = |mutated: Vec<Instruction>| {
            let request = swap_request(case);
            let quote = swap_quote(case, compile(payer, &mutated, &tables));
            verify_swap(case, &request, &quote, &tables)
        };

        let mut extra = ixs.clone();
        extra[2]
            .accounts
            .push(AccountMeta::new(Pubkey::new_unique(), false));
        check(extra).unwrap();

        let mut too_many = ixs.clone();
        while too_many[2].accounts.len() <= 18 + MAX_VAULT_SWAP_ROUTE_ACCOUNTS {
            too_many[2]
                .accounts
                .push(AccountMeta::new_readonly(Pubkey::new_unique(), false));
        }
        assert!(matches!(check(too_many), Err(VaultIntentError::Route(_))));

        let mut policy = ixs.clone();
        policy[2]
            .accounts
            .push(AccountMeta::new(vault.addresses.policy, false));
        assert!(matches!(check(policy), Err(VaultIntentError::Route(_))));

        let mut payer_route = ixs.clone();
        payer_route[2]
            .accounts
            .push(AccountMeta::new_readonly(payer, false));
        assert!(matches!(
            check(payer_route),
            Err(VaultIntentError::Route(_))
        ));

        let mut signer = ixs.clone();
        signer[2]
            .accounts
            .push(AccountMeta::new_readonly(Pubkey::new_unique(), true));
        assert!(check(signer).is_err());

        let mut no_custody = ixs.clone();
        no_custody[2].accounts.truncate(18 + 1);
        assert!(matches!(check(no_custody), Err(VaultIntentError::Route(_))));

        let mut prices = ixs.clone();
        prices[2].accounts[12].pubkey = Pubkey::new_unique();
        assert!(matches!(
            check(prices),
            Err(VaultIntentError::Account {
                index: 2,
                position: 12,
                ..
            })
        ));
    }
}

#[test]
fn key_repeated_in_static_keys_and_lookup_table_is_rejected() {
    let case = cases()
        .into_iter()
        .find(|case| case["kind"] == "swap")
        .unwrap();
    let (payer, ixs) = decoded(&case);
    let mut table = table(&case);
    table.addresses.push(payer);
    let index = u8::try_from(table.addresses.len() - 1).unwrap();
    let mut message = v0::Message::try_compile(
        &payer,
        &ixs,
        std::slice::from_ref(&table),
        Hash::new_from_array([9; 32]),
    )
    .unwrap();
    assert_eq!(message.account_keys[0], payer);
    message.address_table_lookups[0]
        .writable_indexes
        .push(index);
    assert_eq!(
        verify(&case, &encode(VersionedMessage::V0(message)), &[table]),
        Err(VaultIntentError::Malformed("duplicate account key"))
    );
}

#[test]
fn malformed_transactions_are_rejected() {
    let case = &cases()[0];
    let tables = [table(case)];
    let bytes = STANDARD.decode(served_tx(case)).unwrap();
    assert_eq!(
        verify(case, "%%", &tables),
        Err(VaultIntentError::InvalidBase64)
    );
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(matches!(
        verify(case, &STANDARD.encode(trailing), &tables),
        Err(VaultIntentError::Malformed(_))
    ));
    for cut in [1, 64, 66, 100, bytes.len() - 1] {
        assert!(verify(case, &STANDARD.encode(&bytes[..cut]), &tables).is_err());
    }
    let mut version = bytes.clone();
    version[65] = 0x81;
    assert_eq!(
        verify(case, &STANDARD.encode(version), &tables),
        Err(VaultIntentError::UnsupportedVersion(1))
    );
    for header in 66..69 {
        let mut changed = bytes.clone();
        changed[header] ^= 1;
        assert!(verify(case, &STANDARD.encode(changed), &tables).is_err());
    }
    assert_eq!(
        served_lookup_table_keys(&served_tx(case)).unwrap(),
        vec![tables[0].key]
    );
}

fn unsigned_tx(signers: &[Pubkey], extra_keys: &[Pubkey], v0: bool) -> String {
    let mut bytes = vec![signers.len() as u8];
    bytes.extend(std::iter::repeat_n(0u8, 64 * signers.len()));
    if v0 {
        bytes.push(0x80);
    }
    bytes.extend([signers.len() as u8, 0, extra_keys.len() as u8]);
    bytes.push((signers.len() + extra_keys.len()) as u8);
    for key in signers.iter().chain(extra_keys) {
        bytes.extend_from_slice(key.as_ref());
    }
    bytes.extend([3u8; 32]);
    bytes.push(0);
    if v0 {
        bytes.push(0);
    }
    STANDARD.encode(bytes)
}

fn decoded_transaction(tx_base64: &str) -> VerifiedTransaction {
    ServedMessage::decode(tx_base64, &[]).unwrap().transaction
}

#[test]
fn signing_fills_only_the_signer_slot_for_v0_and_legacy() {
    use crate::orders::BytesSigner;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let signer = BytesSigner::from_secret([4u8; 32]);
    let peer = BytesSigner::from_secret([5u8; 32]);
    let verify = |bytes: &[u8], slot: usize, signer: &BytesSigner| {
        let signature = Signature::from_bytes(
            bytes[1 + 64 * slot..1 + 64 * (slot + 1)]
                .try_into()
                .unwrap(),
        );
        VerifyingKey::from_bytes(&signer.pubkey_bytes())
            .unwrap()
            .verify(&bytes[129..], &signature)
            .unwrap();
    };
    for v0 in [true, false] {
        let tx = unsigned_tx(
            &[
                Pubkey::new_from_array(peer.pubkey_bytes()),
                Pubkey::new_from_array(signer.pubkey_bytes()),
            ],
            &[Pubkey::new_from_array([7u8; 32])],
            v0,
        );
        let peer_signed = decoded_transaction(&tx).sign(&peer).unwrap();
        let peer_bytes = STANDARD.decode(&peer_signed).unwrap();
        let signed = decoded_transaction(&peer_signed).sign(&signer).unwrap();
        let bytes = STANDARD.decode(&signed).unwrap();
        verify(&bytes, 0, &peer);
        verify(&bytes, 1, &signer);
        assert_eq!(&bytes[1..65], &peer_bytes[1..65], "peer slot untouched");
        assert_eq!(&bytes[129..], &peer_bytes[129..], "message untouched");
    }
}

#[test]
fn signing_rejects_a_transaction_that_does_not_require_the_signer() {
    let signer = crate::orders::BytesSigner::from_secret([4u8; 32]);
    let key = Pubkey::new_from_array(signer.pubkey_bytes());
    let tx = unsigned_tx(&[Pubkey::new_from_array([5u8; 32])], &[key], true);
    assert_eq!(
        decoded_transaction(&tx).sign(&signer),
        Err(VaultIntentError::SignerNotRequired(key))
    );
}
