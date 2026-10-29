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
use crate::chain::vault::{
    MAX_VAULT_SWAP_ROUTE_ACCOUNTS, PriceRoute, VaultManager, VaultManagerArgs, VaultStagedSlot,
    VaultStagingState, VaultTradeTerms, pyth_price_feed_account,
};
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

fn keeper(case: &Value) -> Pubkey {
    key(&case["keeper"])
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
        second_leg_generation: 0,
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
        side: crate::types::TakerSide::Sell,
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
        vault_pda: manager(case, "swapper").addresses.vault.into(),
        mode: serde_json::from_value(swap["mode"].clone()).unwrap_or(VaultSwapMode::Rebalance),
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
        mode: serde_json::from_value(swap["mode"].clone()).unwrap_or(VaultSwapMode::Rebalance),
        tx_base64,
        last_valid_block_height: 1,
        in_amount: match case["staging"]["amount_in"].as_u64() {
            Some(staged) => staged,
            None => num(&swap["amount"]),
        }
        .to_string(),
        quoted_out_amount: num(&swap["quoted_out_amount"]).to_string(),
        minimum_out_amount: num(&swap["minimum_out_amount"]).to_string(),
        valid_until: num(&swap["valid_until"]).to_string(),
        staging: serde_json::from_value(case["staging"]["mode"].clone())
            .ok()
            .map(|mode: String| match mode.as_str() {
                "stage" => VaultStagingStep::Stage,
                _ => VaultStagingStep::Unstage,
            }),
    }
}

const SHARDS: [u16; 2] = [0, 1];

fn is_swap(case: &Value) -> bool {
    case["kind"] == "swap" || case["kind"] == "staging_swap"
}

fn header_len(case: &Value) -> usize {
    if case["staging"].is_object() { 36 } else { 27 }
}

/// The staging mint's route, if the case is a staging half.
fn staging_routes(case: &Value) -> Vec<PriceRoute> {
    case["staging"]["route"]
        .as_str()
        .map(|route| PriceRoute::parse(&hex::decode(route).unwrap()).unwrap())
        .into_iter()
        .collect()
}

fn price_routes(case: &Value) -> (PriceRoute, PriceRoute) {
    let route = |name: &str| {
        PriceRoute::parse(&hex::decode(case["swap"][name].as_str().unwrap()).unwrap()).unwrap()
    };
    assert_eq!(case["swap"]["shards"], serde_json::json!(SHARDS));
    (route("main_route"), route("second_route"))
}

/// Fixed accounts plus the served price section.
fn swap_prefix(case: &Value) -> usize {
    let (main, second) = price_routes(case);
    let Some(staging) = staging_routes(case).pop() else {
        return 14
            + manager(case, "swapper")
                .price_section(&main, &second, 0)
                .unwrap()
                .len();
    };
    let leg = if case["staging"]["leg"] == "main" {
        main.clone()
    } else {
        second
    };
    let recovery_tail = if case["swap"]["mode"] == "recovery" {
        2 + main.accounts(Pubkey::default(), 1).len()
    } else {
        0
    };
    14 + leg.accounts(Pubkey::default(), 0).len()
        + staging.accounts(Pubkey::default(), 1).len()
        + recovery_tail
}

/// Slot before the request: empty at `generation - 1` to stage, holding the amount to unstage.
fn fixture_slot(case: &Value) -> VaultStagingState {
    let staging = &case["staging"];
    if !staging.is_object() {
        return VaultStagingState::default();
    }
    let generation = num(&staging["generation"]);
    if staging["mode"] == "stage" {
        return VaultStagingState {
            generation: generation - 1,
            open: None,
        };
    }
    VaultStagingState {
        generation,
        open: Some(VaultStagedSlot {
            mint: key(&staging["mint"]),
            from_leg: VaultLeg::Main,
            amount: num(&staging["amount_in"]),
            staged_at: 1,
        }),
    }
}

fn swap_intent<'a>(
    vault: &'a VaultManager,
    request: &'a SwapQuoteData,
    quote: &'a SwapQuoteResultData,
    routes: &'a (PriceRoute, PriceRoute),
    staging: &'a [PriceRoute],
    shards: &'a [u16],
    min_output_floor: u64,
) -> VaultSwapIntent<'a> {
    VaultSwapIntent {
        vault,
        request,
        quote,
        price_routes: PairPriceRoutes {
            main: &routes.0,
            second: &routes.1,
            staging,
            shards,
        },
        min_output_floor,
        account: vault.test_account(VaultStagingState::default()),
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
    let routes = price_routes(case);
    let staging = staging_routes(case);
    verify_vault_swap(
        &VaultSwapIntent {
            account: vault.test_account(fixture_slot(case)),
            ..swap_intent(
                &vault,
                request,
                quote,
                &routes,
                &staging,
                &SHARDS,
                min_output_floor,
            )
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
                    keeper: keeper(case),
                    writer: &writer,
                    maker_owner: key(&case["maker_owner"]),
                    maker_quote_signer: key(&case["position"]["quote_signer"]),
                    terms: VaultSaleTerms::Physical(terms),
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
                    keeper: keeper(case),
                    writer: &writer,
                    holder: &holder,
                    terms: VaultSaleTerms::Physical(terms),
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
                    keeper: keeper(case),
                    holder: &holder,
                    taker: key(&case["taker"]),
                    terms: VaultSaleTerms::Physical(terms),
                },
                tables,
                BOUNDS,
            )
            .map(|_| ())
        }
        "swap" | "staging_swap" => verify_swap(
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
            side: crate::types::TakerSide::Sell,
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

/// The swap follows the compute budget and any setup.
fn swap_index(case: &Value) -> usize {
    2 + usize::try_from(num(&case["swap"]["setup"])).unwrap()
}

/// Bytes whose change must be rejected; stake-pool accounts are the stake-pool program's to check.
fn route_start(case: &Value, index: usize, ix: &Instruction) -> (usize, usize) {
    if is_swap(case) && index == swap_index(case) {
        (swap_prefix(case), header_len(case))
    } else if STAKE_POOL_PROGRAM_IDS.contains(&ix.program_id) {
        (2, ix.data.len())
    } else {
        (ix.accounts.len(), ix.data.len())
    }
}

#[test]
fn every_served_fixture_verifies() {
    let cases = cases();
    assert_eq!(cases.len(), 21);
    for case in &cases {
        verify(case, &served_tx(case), &[table(case)])
            .unwrap_or_else(|error| panic!("{} rejected: {error}", case["kind"]));
    }
}

#[test]
fn vault_open_fixtures_record_bounded_v0_alt_wire_sizes() {
    for case in cases().iter().filter(|case| case["position"].is_object()) {
        let bytes = STANDARD.decode(served_tx(case)).unwrap();
        let recorded = usize::try_from(num(&case["wire_bytes"])).unwrap();
        assert_eq!(
            bytes.len(),
            recorded,
            "{} fixture size drifted",
            case["kind"]
        );
        assert!(
            recorded <= MAX_TRANSACTION_WIRE_BYTES,
            "{} is {recorded} bytes",
            case["kind"]
        );
        let served = ServedMessage::decode(&served_tx(case), &[table(case)]).unwrap();
        assert!(served.is_version_zero, "{} is not v0", case["kind"]);
        assert!(
            served.has_lookup_table,
            "{} has no address lookup table",
            case["kind"]
        );
    }
}

#[test]
fn recompiled_v0_messages_verify_and_legacy_opens_are_rejected() {
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
        if is_swap(case) {
            verify(case, &legacy, &[]).unwrap();
        } else {
            assert_eq!(
                verify(case, &legacy, &[]),
                Err(VaultIntentError::OpenRequiresVersionZero)
            );
        }
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
                // A hop may write a price-section account it carries (prices are read first).
                let hop_writable = is_swap(case)
                    && index == swap_index(case)
                    && position >= 14
                    && ix.accounts[route_accounts..]
                        .iter()
                        .any(|meta| meta.pubkey == ix.accounts[position].pubkey);
                if replaced[index].accounts[position].pubkey != payer && !hop_writable {
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
            let optional_setup = is_swap(case) && (2..swap_index(case)).contains(&index);
            assert_eq!(
                verify(case, &compile(payer, &missing, &tables), &tables).is_ok(),
                optional_setup
            );
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
fn vault_trade_builder_refuses_taker_buy_before_constructing_instructions() {
    let case = cases()
        .into_iter()
        .find(|case| case["kind"] == "vault_taker")
        .expect("vault taker fixture");
    let mut trade_terms = terms(&case);
    trade_terms.side = crate::types::TakerSide::Buy;

    assert!(matches!(
        manager(&case, "writer").open_position_as_taker(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            &trade_terms,
        ),
        Err(VaultChainError::InvalidInput("taker_buy_unsupported"))
    ));
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
fn swap_refuses_a_manager_from_before_a_second_leg_roll() {
    let case = cases()
        .into_iter()
        .find(|case| case["kind"] == "swap")
        .unwrap();
    let tables = [table(&case)];
    let request = swap_request(&case);
    let quote = swap_quote(&case, served_tx(&case));
    let vault = manager(&case, "swapper");
    let routes = price_routes(&case);
    let mut rolled = vault.test_account(VaultStagingState::default());
    rolled.second_leg_generation = 1;
    assert!(matches!(
        verify_vault_swap(
            &VaultSwapIntent {
                account: rolled,
                ..swap_intent(&vault, &request, &quote, &routes, &[], &SHARDS, 1)
            },
            &tables,
            BOUNDS,
        ),
        Err(VaultIntentError::Intent(VaultChainError::SecondLegRolled))
    ));
}

#[test]
fn swap_terms_follow_the_request() {
    let tables_of = |case: &Value| [table(case)];
    for case in cases().iter().filter(|case| case["kind"] == "swap") {
        let tables = tables_of(case);
        let request = swap_request(case);
        let quote = swap_quote(case, served_tx(case));
        verify_swap(case, &request, &quote, &tables).unwrap();
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
            Err(VaultIntentError::Route(_))
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
        let vault = manager(case, "swapper");
        let routes = price_routes(case);
        let with =
            |routes: &(PriceRoute, PriceRoute), shards: &[u16], quote: &SwapQuoteResultData| {
                verify_vault_swap(
                    &swap_intent(&vault, &request, quote, routes, &[], shards, 1),
                    &tables,
                    BOUNDS,
                )
                .map(|_| ())
            };
        assert_eq!(
            with(&routes, &[2], &quote),
            Err(VaultIntentError::PriceSection("main"))
        );
        let crossed = (routes.1.clone(), routes.0.clone());
        assert!(matches!(
            with(&crossed, &SHARDS, &quote),
            Err(VaultIntentError::Intent(VaultChainError::InvalidInput(_)))
        ));

        let (payer, mut ixs) = decoded(case);
        let feed = routes
            .1
            .bricks
            .iter()
            .find_map(|brick| match brick {
                crate::chain::vault::PriceBrick::Pyth(pyth) => Some(pyth.feed_id),
                _ => None,
            })
            .unwrap();
        let s = swap_index(case);
        let second_feed = ixs[s].accounts[..swap_prefix(case)]
            .iter()
            .rposition(|meta| meta.pubkey == pyth_price_feed_account(&feed, 1))
            .unwrap();
        ixs[s].accounts[second_feed].pubkey = pyth_price_feed_account(&feed, 2);
        let other_shard = swap_quote(case, compile(payer, &ixs, &tables));
        assert_eq!(
            with(&routes, &SHARDS, &other_shard),
            Err(VaultIntentError::PriceSection("second"))
        );
        ixs[s].accounts[second_feed].pubkey = pyth_price_feed_account(&feed, 1);
        let shard_one = swap_quote(case, compile(payer, &ixs, &tables));
        with(&routes, &SHARDS, &shard_one).unwrap();
        assert_eq!(
            with(&routes, &[0], &shard_one),
            Err(VaultIntentError::PriceSection("second"))
        );
    }
}

#[test]
fn staging_swaps_bind_step_leg_and_staging_route() {
    let staged: Vec<Value> = cases()
        .into_iter()
        .filter(|case| case["kind"] == "staging_swap")
        .collect();
    assert_eq!(staged.len(), 3);
    for case in &staged {
        let tables = [table(case)];
        let request = swap_request(case);
        let quote = swap_quote(case, served_tx(case));
        let step = quote.staging.unwrap();
        verify_swap(case, &request, &quote, &tables).unwrap();

        let mut unstaged = quote.clone();
        unstaged.staging = None;
        assert!(verify_swap(case, &request, &unstaged, &tables).is_err());
        let mut flipped = quote.clone();
        flipped.staging = Some(match step {
            VaultStagingStep::Stage => VaultStagingStep::Unstage,
            _ => VaultStagingStep::Stage,
        });
        assert!(verify_swap(case, &request, &flipped, &tables).is_err());
        let mut reversed = request.clone();
        reversed.direction = match request.direction {
            VaultSwapDirection::MainToSecond => VaultSwapDirection::SecondToMain,
            _ => VaultSwapDirection::MainToSecond,
        };
        assert!(verify_swap(case, &reversed, &quote, &tables).is_err());

        let vault = manager(case, "swapper");
        let routes = price_routes(case);
        assert_eq!(
            verify_vault_swap(
                &VaultSwapIntent {
                    account: vault.test_account(fixture_slot(case)),
                    ..swap_intent(&vault, &request, &quote, &routes, &[], &SHARDS, 1)
                },
                &tables,
                BOUNDS,
            )
            .map(|_| ()),
            Err(VaultIntentError::PriceSection("staging"))
        );

        // Stage fills the staging mint (leg floor moot); unstage spends into the floored leg.
        let mut other_amount = request.clone();
        other_amount.amount = "1".into();
        let floored = verify_swap_with_floor(case, &request, &quote, &tables, u64::MAX);
        match step {
            VaultStagingStep::Stage => {
                assert_eq!(
                    verify_swap(case, &other_amount, &quote, &tables),
                    Err(VaultIntentError::Terms("quoted input differs from request"))
                );
                floored.unwrap();
            }
            _ => {
                assert_ne!(quote.in_amount, request.amount);
                verify_swap(case, &other_amount, &quote, &tables).unwrap();
                assert_eq!(
                    floored,
                    Err(VaultIntentError::Terms(
                        "minimum output is below the operator's floor"
                    ))
                );
            }
        }
    }
}

#[test]
fn staging_halves_name_the_vaults_slot() {
    for case in cases().iter().filter(|case| case["kind"] == "staging_swap") {
        let tables = [table(case)];
        let request = swap_request(case);
        let quote = swap_quote(case, served_tx(case));
        let vault = manager(case, "swapper");
        let routes = price_routes(case);
        let staging = staging_routes(case);
        let slot = fixture_slot(case);
        let with = |slot: VaultStagingState| {
            verify_vault_swap(
                &VaultSwapIntent {
                    account: vault.test_account(slot),
                    ..swap_intent(&vault, &request, &quote, &routes, &staging, &SHARDS, 1)
                },
                &tables,
                BOUNDS,
            )
            .map(|_| ())
        };
        with(slot).unwrap();
        let stale = Err(VaultIntentError::Terms(
            "staging generation does not name the vault's slot",
        ));
        for generation in [slot.generation - 1, slot.generation + 1] {
            assert_eq!(with(VaultStagingState { generation, ..slot }), stale);
        }
        let open = VaultStagedSlot {
            mint: key(&case["staging"]["mint"]),
            from_leg: VaultLeg::Second,
            amount: num(&case["staging"]["amount_in"]),
            staged_at: 1,
        };
        if quote.staging == Some(VaultStagingStep::Stage) {
            assert_eq!(
                with(VaultStagingState {
                    open: Some(open),
                    ..slot
                }),
                Err(VaultIntentError::Terms(
                    "a stage needs an empty staging slot"
                ))
            );
            assert_eq!(
                with(VaultStagingState {
                    generation: u64::MAX,
                    open: None
                }),
                stale
            );
            continue;
        }
        assert_eq!(
            with(VaultStagingState { open: None, ..slot }),
            Err(VaultIntentError::Terms("nothing is staged to unstage"))
        );
        for amount in [open.amount - 1, open.amount + 1] {
            assert_eq!(
                with(VaultStagingState {
                    open: Some(VaultStagedSlot { amount, ..open }),
                    ..slot
                }),
                Err(VaultIntentError::Terms(
                    "an unstage must spend the whole staged amount"
                ))
            );
        }
        assert_eq!(
            with(VaultStagingState {
                open: Some(VaultStagedSlot {
                    mint: Pubkey::new_unique(),
                    ..open
                }),
                ..slot
            }),
            Err(VaultIntentError::Terms(
                "unstage names another staging mint"
            ))
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
        let (prefix, s) = (swap_prefix(case), swap_index(case));
        ixs[s] = manager(case, "swapper")
            .swap(
                kind,
                &VaultSwapPlan {
                    price_section: ixs[s].accounts[14..prefix].to_vec(),
                    amount_in: num(&case["swap"]["amount"]),
                    minimum_out: 1,
                    valid_until: num(&case["swap"]["valid_until"]),
                    hops: served_hops(
                        ixs[s].data[header_len(case) - 1],
                        &ixs[s].data[header_len(case)..],
                        &ixs[s].accounts[prefix..],
                    )
                    .unwrap(),
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

/// Re-encodes the served swap with `hops`, skipping the builder's own checks.
fn with_hops(case: &Value, swap: &Instruction, hops: &[VaultSwapHop]) -> Instruction {
    let prefix = swap_prefix(case);
    let mut ix = swap.clone();
    ix.accounts.truncate(prefix);
    let header = header_len(case);
    ix.data.truncate(header);
    ix.data[header - 1] = u8::try_from(hops.len()).unwrap();
    for hop in hops {
        ix.data.push(u8::try_from(hop.accounts.len()).unwrap());
        ix.data
            .extend_from_slice(&u16::try_from(hop.data.len()).unwrap().to_le_bytes());
        match hop.amount_offset {
            Some(amount) => {
                ix.data.extend_from_slice(&amount.offset.to_le_bytes());
                ix.data.extend_from_slice(&amount.num.to_le_bytes());
                ix.data.extend_from_slice(&amount.den.to_le_bytes());
            }
            None => ix.data.extend_from_slice(&u16::MAX.to_le_bytes()),
        }
        ix.data.extend_from_slice(&hop.data);
        ix.accounts.extend([
            AccountMeta::new_readonly(hop.program, false),
            AccountMeta::new(hop.source, false),
            AccountMeta::new(hop.destination, false),
        ]);
        ix.accounts.extend(hop.accounts.iter().cloned());
    }
    ix
}

#[test]
fn swap_hops_are_confined_and_bounded() {
    for case in cases().iter().filter(|case| is_swap(case)) {
        let (payer, ixs) = decoded(case);
        let tables = [table(case)];
        let vault = manager(case, "swapper");
        let (prefix, s) = (swap_prefix(case), swap_index(case));
        let hops = served_hops(
            ixs[s].data[header_len(case) - 1],
            &ixs[s].data[header_len(case)..],
            &ixs[s].accounts[prefix..],
        )
        .unwrap();
        assert_eq!(hops.len() as u64, num(&case["swap"]["hops"]));
        assert_eq!(with_hops(case, &ixs[s], &hops), ixs[s]);
        let check = |hops: &[VaultSwapHop]| {
            let mut mutated = ixs.clone();
            mutated[s] = with_hops(case, &ixs[s], hops);
            let quote = swap_quote(case, compile(payer, &mutated, &tables));
            verify_swap(case, &swap_request(case), &quote, &tables)
        };
        let raw = |mutate: &dyn Fn(&mut Instruction)| {
            let mut mutated = ixs.clone();
            mutate(&mut mutated[s]);
            let quote = swap_quote(case, compile(payer, &mutated, &tables));
            verify_swap(case, &swap_request(case), &quote, &tables)
        };
        let last = hops.len() - 1;
        let edit = |index: usize, change: &dyn Fn(&mut VaultSwapHop)| {
            let mut hops = hops.clone();
            change(&mut hops[index]);
            hops
        };

        check(&edit(last, &|hop| {
            hop.accounts
                .push(AccountMeta::new(Pubkey::new_unique(), false));
            hop.data.push(7);
        }))
        .unwrap();
        assert!(matches!(
            raw(&|ix| ix
                .accounts
                .push(AccountMeta::new(Pubkey::new_unique(), false))),
            Err(VaultIntentError::AccountCount { .. })
        ));
        assert!(matches!(
            raw(&|ix| ix.data.push(0)),
            Err(VaultIntentError::Data { .. })
        ));
        assert_eq!(
            raw(&|ix| ix.data[header_len(case) - 1] += 1),
            Err(VaultIntentError::Route("swap hops are truncated"))
        );
        assert!(matches!(
            raw(&|ix| ix.data[header_len(case) - 1] = 0),
            Err(VaultIntentError::Route(_))
        ));
        assert!(matches!(
            raw(&|ix| ix.accounts[13].pubkey = Pubkey::new_unique()),
            Err(VaultIntentError::Account { index, position: 13, .. }) if index == s
        ));
        assert!(matches!(
            check(&edit(0, &|hop| {
                while hop.accounts.len() <= MAX_VAULT_SWAP_ROUTE_ACCOUNTS {
                    hop.accounts
                        .push(AccountMeta::new_readonly(Pubkey::new_unique(), false));
                }
            })),
            Err(VaultIntentError::Route(_))
        ));
        let with_offset = |index: usize, offset: u16, num: u64, den: u64| {
            check(&edit(index, &|hop| {
                hop.data.resize(9, 0);
                hop.amount_offset = Some(VaultSwapAmountOffset { offset, num, den });
            }))
        };
        assert!(matches!(
            with_offset(0, 0, 1, 1),
            Err(VaultIntentError::Route(_))
        ));
        if hops.len() > 1 {
            with_offset(1, 1, 1_081_430_937_307, 1_000_000_000_000).unwrap();
            for (offset, num, den) in [(2, 1, 1), (1, 0, 1), (1, 1, 0)] {
                assert!(matches!(
                    with_offset(1, offset, num, den),
                    Err(VaultIntentError::Route(_))
                ));
            }
        }
        // Exponent `merge` burns the input leg's PT mint; a Generic SY redeem the staging mint.
        let burned = if case["staging"].is_object() {
            ixs[s].accounts[9].pubkey
        } else if swap_request(case).direction == VaultSwapDirection::MainToSecond {
            ixs[s].accounts[7].pubkey
        } else {
            ixs[s].accounts[8].pubkey
        };
        // Setup instructions may share the served hop's accounts, so the shape goes in front.
        let shaped = edit(0, &|hop| {
            let shape = [
                AccountMeta::new(vault.addresses.vault, false),
                AccountMeta::new(hop.destination, false),
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new(hop.source, false),
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new(burned, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ];
            hop.accounts.splice(0..0, shape);
        });
        check(&shaped).unwrap();
        let mut policy = shaped.clone();
        policy[0]
            .accounts
            .push(AccountMeta::new(vault.addresses.policy, false));
        assert_eq!(
            check(&policy),
            Err(VaultIntentError::Route(
                "route makes a read-only account writable"
            ))
        );
        for pinned in [
            vault.addresses.policy,
            vault.addresses.global_config,
            vault.addresses.swap_venues,
            vault.addresses.manager_stake_ata,
            hops[0].program,
        ] {
            assert_eq!(
                check(&edit(last, &|hop| hop
                    .accounts
                    .push(AccountMeta::new(pinned, false)))),
                Err(VaultIntentError::Route(
                    "route makes a read-only account writable"
                ))
            );
        }
        // The initial pair is read before hops; recovery's MAIN tail stays pinned.
        let price_end = if case["swap"]["mode"] == "recovery" {
            prefix - 2 - price_routes(case).0.accounts(Pubkey::default(), 1).len()
        } else {
            prefix
        };
        for priced in [
            ixs[s].accounts[14].pubkey,
            ixs[s].accounts[price_end - 1].pubkey,
        ] {
            check(&edit(last, &|hop| {
                hop.accounts.push(AccountMeta::new(priced, false))
            }))
            .unwrap();
        }
        assert_eq!(
            check(&edit(0, &|hop| hop
                .accounts
                .push(AccountMeta::new_readonly(payer, false)))),
            Err(VaultIntentError::Route("route uses the fee payer"))
        );
        assert!(
            check(&edit(0, &|hop| hop
                .accounts
                .push(AccountMeta::new_readonly(Pubkey::new_unique(), true))))
            .is_err()
        );
        assert!(matches!(
            check(&edit(last, &|hop| hop
                .accounts
                .retain(|meta| meta.pubkey != hop.destination))),
            Err(VaultIntentError::Route(_))
        ));
        assert!(matches!(
            check(&edit(last, &|hop| hop.destination = hop.source)),
            Err(VaultIntentError::Route(_))
        ));
        assert!(matches!(
            check(&edit(0, &|hop| hop.data.clear())),
            Err(VaultIntentError::Route(_))
        ));
        let mut five = hops.clone();
        while five.len() < 5 {
            five.insert(0, hops[0].clone());
        }
        assert!(matches!(check(&five), Err(VaultIntentError::Route(_))));

        let vault_key = vault.addresses.vault;
        let scratch = ata(&vault_key, &Pubkey::new_unique(), &TOKEN_PROGRAM_ID);
        check(&edit(0, &|hop| {
            hop.accounts.push(AccountMeta::new(scratch, false))
        }))
        .unwrap();
        if last > 0 {
            assert_eq!(
                check(&edit(last, &|hop| hop
                    .accounts
                    .push(AccountMeta::new(hops[0].source, false)))),
                Err(VaultIntentError::Route(
                    "a leg ATA may appear only as its hop's source or destination"
                ))
            );
        }
        let share_mint = vault.args.share_mint;
        for share in [share_mint, ata(&vault_key, &share_mint, &TOKEN_PROGRAM_ID)] {
            assert_eq!(
                check(&edit(0, &|hop| hop
                    .accounts
                    .push(AccountMeta::new_readonly(share, false)))),
                Err(VaultIntentError::Route(
                    "route touches the vault share mint or its share account"
                ))
            );
        }

        let first_leg = if case["staging"]["leg"] == "second" {
            "second"
        } else {
            "main"
        };
        assert_eq!(
            raw(&|ix| ix.accounts[14].pubkey = Pubkey::new_unique()),
            Err(VaultIntentError::PriceSection(first_leg))
        );
    }
}

#[test]
fn swap_setup_is_vault_scratch_or_a_pinned_pool_update() {
    let case = cases()
        .into_iter()
        .find(|case| {
            case["swap"]["setup"]
                .as_u64()
                .is_some_and(|setup| setup > 0)
        })
        .unwrap();
    let (payer, ixs) = decoded(&case);
    let tables = [table(&case)];
    let vault = manager(&case, "swapper");
    let s = swap_index(&case);
    let (ata_setup, pool_setup) = (2, s - 1);
    assert_eq!(ixs[ata_setup].program_id, ASSOCIATED_TOKEN_PROGRAM_ID);
    assert!(STAKE_POOL_PROGRAM_IDS.contains(&ixs[pool_setup].program_id));
    let with = |index: usize, ix: Instruction| {
        let mut mutated = ixs.clone();
        mutated[index] = ix;
        verify(&case, &compile(payer, &mutated, &tables), &tables)
    };
    let scratch_mint = ixs[ata_setup].accounts[3].pubkey;
    let vault_key = vault.addresses.vault;

    with(ata_setup, ixs[ata_setup].clone()).unwrap();
    assert_eq!(
        with(
            ata_setup,
            create_ata_idempotent(
                payer,
                vault_key,
                Pubkey::new_unique(),
                TOKEN_2022_PROGRAM_ID,
            ),
        ),
        Err(VaultIntentError::Setup(
            "creates an account the route does not use"
        ))
    );
    let mut twice = ixs.clone();
    twice.insert(ata_setup, ixs[ata_setup].clone());
    assert_eq!(
        verify(&case, &compile(payer, &twice, &tables), &tables),
        Err(VaultIntentError::Setup("creates the same account twice"))
    );
    for leg in [
        vault.args.main_mint,
        vault.args.second_mint,
        vault.args.share_mint,
    ] {
        assert_eq!(
            with(
                ata_setup,
                create_ata_idempotent(payer, vault_key, leg, TOKEN_PROGRAM_ID)
            ),
            Err(VaultIntentError::Setup(
                "creates an account for a vault leg or share mint"
            ))
        );
    }
    for (funder, owner) in [
        (payer, Pubkey::new_unique()),
        (Pubkey::new_unique(), vault_key),
    ] {
        assert!(matches!(
            with(
                ata_setup,
                create_ata_idempotent(funder, owner, scratch_mint, TOKEN_PROGRAM_ID)
            ),
            Err(VaultIntentError::Account { .. })
        ));
    }
    assert_eq!(
        with(
            ata_setup,
            create_ata_idempotent(payer, vault_key, scratch_mint, Pubkey::new_unique())
        ),
        Err(VaultIntentError::Setup("token program is not SPL Token"))
    );

    let pool_update = |program: Pubkey, pool: Pubkey| {
        let mut ix = ixs[pool_setup].clone();
        ix.program_id = program;
        ix.accounts[0].pubkey = pool;
        ix.accounts[1].pubkey =
            Pubkey::find_program_address(&[pool.as_ref(), b"withdraw"], &program).0;
        ix
    };
    let pool = ixs[pool_setup].accounts[0].pubkey;
    with(pool_setup, pool_update(STAKE_POOL_PROGRAM_IDS[0], pool)).unwrap();
    assert_eq!(
        with(
            pool_setup,
            pool_update(STAKE_POOL_PROGRAM_IDS[1], Pubkey::new_unique())
        ),
        Err(VaultIntentError::Setup(
            "updates a stake pool the vault's routes do not pin"
        ))
    );
    assert_eq!(
        with(pool_setup, pool_update(Pubkey::new_unique(), pool)),
        Err(VaultIntentError::Setup(
            "unexpected instruction before the swap"
        ))
    );
    let mut wrong_authority = ixs[pool_setup].clone();
    wrong_authority.accounts[1].pubkey = Pubkey::new_unique();
    assert!(matches!(
        with(pool_setup, wrong_authority),
        Err(VaultIntentError::Account { .. })
    ));

    for setup in [ata_setup, pool_setup] {
        let mut after = ixs.clone();
        let moved = after.remove(setup);
        after.push(moved);
        assert!(matches!(
            verify(&case, &compile(payer, &after, &tables), &tables),
            Err(VaultIntentError::InstructionCount { .. })
        ));
    }
}

#[test]
fn key_repeated_in_static_keys_and_lookup_table_is_rejected() {
    let case = cases().into_iter().find(is_swap).unwrap();
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
    let message_start = 1 + 64 * usize::from(bytes[0]);
    version[message_start] = 0x81;
    assert_eq!(
        verify(case, &STANDARD.encode(version), &tables),
        Err(VaultIntentError::UnsupportedVersion(1))
    );
    for header in message_start + 1..message_start + 4 {
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

#[test]
fn recovery_stage_pins_main_valuation_and_hop_program() {
    let case = cases()
        .into_iter()
        .find(|c| c["swap"]["mode"] == "recovery")
        .unwrap();
    let (payer, ixs) = decoded(&case);
    let tables = [table(&case)];
    let request = swap_request(&case);
    let s = swap_index(&case);
    let prefix = swap_prefix(&case);
    let main = price_routes(&case).0;
    let tail = prefix - 2 - main.accounts(Pubkey::default(), 1).len();
    assert_eq!(&ixs[s].data[..3], &[76, 2, 1]);
    let check = |ixs: &[Instruction]| {
        verify_swap(
            &case,
            &request,
            &swap_quote(&case, compile(payer, ixs, &tables)),
            &tables,
        )
    };
    check(&ixs).unwrap();
    for position in tail..=prefix {
        let mut changed = ixs.clone();
        changed[s]
            .accounts
            .push(AccountMeta::new(ixs[s].accounts[position].pubkey, false));
        changed[s].data[header_len(&case)] += 1;
        if position == tail {
            check(&changed).unwrap(); // MAIN custody is writable by design.
        } else {
            assert_eq!(
                check(&changed),
                Err(VaultIntentError::Route(
                    "route makes a read-only account writable"
                ))
            );
        }
    }
    let mut downgraded = ixs.clone();
    downgraded[s].data[1] = 0;
    assert!(check(&downgraded).is_err());
    let mut truncated = ixs.clone();
    truncated[s].accounts.truncate(tail + 1);
    assert!(check(&truncated).is_err());
}
