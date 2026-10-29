use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::{VersionedMessage, v0};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;

use super::served::create_ata_idempotent;
use super::*;
use crate::chain::ix::{TOKEN_PROGRAM_ID, derive_associated_token_address as ata};
use crate::orders::{BytesSigner, SignerLike};
use crate::spreads::SpreadType;
use crate::types::{OrderId, TakerSide};
use crate::ws::types::SponsoredTxToSignData;

const BOUNDS: ComputeBudgetBounds = ComputeBudgetBounds {
    max_unit_limit: 400_000,
    max_unit_price_micro_lamports: 1_000,
};
const ORDER_VECTORS: &str = include_str!("../../../tests/fixtures/order_preimage_vectors.json");
const KEEPER: u8 = 11;

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn keeper() -> Pubkey {
    key(KEEPER)
}

fn lookup_table_for_open(
    keeper: Pubkey,
    participant: Pubkey,
    open: &VaultOpenPosition,
    instructions: &[Instruction],
) -> solana_sdk::message::AddressLookupTableAccount {
    let mut static_keys = HashSet::from([
        keeper,
        participant,
        open.addresses.position,
        open.addresses.collateral_ata,
    ]);
    for instruction in instructions {
        static_keys.insert(instruction.program_id);
        static_keys.extend(
            instruction
                .accounts
                .iter()
                .filter(|account| account.is_signer)
                .map(|account| account.pubkey),
        );
    }
    let mut seen = HashSet::new();
    solana_sdk::message::AddressLookupTableAccount {
        key: key(90),
        addresses: instructions
            .iter()
            .flat_map(|instruction| instruction.accounts.iter())
            .map(|account| account.pubkey)
            .filter(|account| !static_keys.contains(account) && seen.insert(*account))
            .collect(),
    }
}

fn manager(vault_id: u8) -> VaultManager {
    VaultManager::new(VaultManagerArgs {
        program_id: Pubkey::new_from_array(crate::ACTA_PROGRAM_ID),
        vault_id: [vault_id; 32],
        actor: key(1),
        delegate_authority: key(2),
        main_mint: key(3),
        second_mint: key(4),
        share_mint: key(5),
        main_token_program: TOKEN_PROGRAM_ID,
        second_token_program: TOKEN_PROGRAM_ID,
        second_leg_generation: 0,
    })
    .unwrap()
}

/// Underlying is the vault's main mint, so its quote leg is `second`.
fn terms(kind: SpreadType) -> VaultSpreadTerms {
    VaultSpreadTerms {
        underlying_mint: key(3),
        quote_mint: key(4),
        expiry_ts: 100,
        kind,
        side: TakerSide::Sell,
        lower_strike: 100,
        upper_strike: 120,
        quantity: 20,
        price: 5,
        valid_until: 90,
        nonce: 1,
    }
}

#[test]
fn vault_spread_routes_use_spread_opcodes_layout_and_quote_leg_custody() {
    let vault = manager(7);
    let holder = manager(8);
    for kind in [SpreadType::CallSpread, SpreadType::PutSpread] {
        let terms = terms(kind);
        let quote_ata = |owner: &Pubkey| {
            crate::chain::ix::derive_associated_token_address(owner, &key(4), &TOKEN_PROGRAM_ID)
        };
        let taker = vault
            .open_spread_as_taker(keeper(), key(9), &terms)
            .unwrap();
        assert_eq!(
            (taker.instruction.data[0], taker.instruction.data.len()),
            (78, 82)
        );
        assert_eq!(taker.instruction.data[1], kind as u8);
        assert_eq!(&taker.instruction.data[74..], &120u64.to_le_bytes());
        assert_eq!(taker.instruction.accounts.len(), 23);
        assert_eq!(
            taker.addresses.collateral_ata,
            quote_ata(&taker.addresses.position)
        );
        assert_eq!(
            taker.order_id,
            terms
                .order_id(&vault.args.program_id, &key(9), &vault.addresses.vault)
                .unwrap()
        );
        let maker = vault
            .open_spread_as_maker(keeper(), key(9), &terms)
            .unwrap();
        assert_eq!(
            (maker.instruction.data[0], maker.instruction.accounts.len()),
            (79, 22)
        );
        assert_eq!(
            maker.instruction.accounts[5].pubkey,
            vault.addresses.vault_second_ata
        );
        let dual = vault
            .open_spread_with_vault_holder(keeper(), &holder, &terms)
            .unwrap();
        assert_eq!(
            (dual.instruction.data[0], dual.instruction.accounts.len()),
            (80, 27)
        );
        assert_eq!(
            dual.instruction.accounts[10].pubkey,
            holder.addresses.vault_second_ata
        );
        let moved = VaultSpreadTerms {
            upper_strike: 121,
            ..terms
        };
        assert_ne!(
            vault
                .open_spread_as_taker(keeper(), key(9), &moved)
                .unwrap()
                .order_id,
            taker.order_id
        );
    }
    assert!(
        vault
            .open_spread_with_vault_holder(keeper(), &vault, &terms(SpreadType::CallSpread))
            .is_err()
    );
    for bad in [
        VaultSpreadTerms {
            price: 20,
            ..terms(SpreadType::CallSpread)
        },
        VaultSpreadTerms {
            side: TakerSide::Buy,
            ..terms(SpreadType::CallSpread)
        },
        VaultSpreadTerms {
            quote_mint: key(6),
            ..terms(SpreadType::CallSpread)
        },
    ] {
        assert!(vault.open_spread_as_maker(keeper(), key(9), &bad).is_err());
    }
}

/// A served vault spread sale in the order tx-service builds it: compute budget,
/// position collateral ATA, the counterparty's quote signature, the open.
pub(crate) fn served_sale(
    keeper: Pubkey,
    open: &VaultOpenPosition,
    quote_signer: &impl SignerLike,
) -> Vec<Instruction> {
    let budget = |data: Vec<u8>| Instruction {
        program_id: solana_sdk::pubkey!("ComputeBudget111111111111111111111111111111"),
        accounts: Vec::new(),
        data,
    };
    let mut ed25519 = vec![1, 0];
    for offset in [48u16, u16::MAX, 16, u16::MAX, 112, 32, u16::MAX] {
        ed25519.extend_from_slice(&offset.to_le_bytes());
    }
    ed25519.extend_from_slice(&quote_signer.pubkey_bytes());
    ed25519.extend_from_slice(&quote_signer.sign_message(&open.order_id));
    ed25519.extend_from_slice(&open.order_id);
    vec![
        budget([&[2][..], &200_000u32.to_le_bytes()].concat()),
        budget([&[3][..], &1u64.to_le_bytes()].concat()),
        create_ata_idempotent(
            keeper,
            open.addresses.position,
            open.addresses.collateral_mint,
            open.addresses.collateral_token_program,
        ),
        Instruction {
            program_id: solana_sdk::pubkey!("Ed25519SigVerify111111111111111111111111111"),
            accounts: Vec::new(),
            data: ed25519,
        },
        open.instruction.clone(),
    ]
}

/// A served vault spread fill: the keeper creates the taker's quote ATA and the
/// position collateral ATA before the vault-maker open.
pub(crate) fn served_fill(
    keeper: Pubkey,
    taker: Pubkey,
    quote_mint: Pubkey,
    quote_token_program: Pubkey,
    open: &VaultOpenPosition,
    quote_signer: &impl SignerLike,
) -> Vec<Instruction> {
    let mut instructions = served_sale(keeper, open, quote_signer);
    instructions.insert(
        2,
        create_ata_idempotent(keeper, taker, quote_mint, quote_token_program),
    );
    instructions
}

pub(crate) fn compile_with_tables(
    keeper: Pubkey,
    ixs: &[Instruction],
    tables: &[solana_sdk::message::AddressLookupTableAccount],
) -> String {
    let message = VersionedMessage::V0(
        v0::Message::try_compile(&keeper, ixs, tables, Hash::new_from_array([9; 32])).unwrap(),
    );
    let signers = usize::from(message.header().num_required_signatures);
    let mut bytes = vec![u8::try_from(signers).unwrap()];
    bytes.extend(std::iter::repeat_n(0u8, signers * 64));
    bytes.extend(message.serialize());
    STANDARD.encode(bytes)
}

/// The shared vector's terms on the test vaults' pair, and its preimage.
fn vector_terms(name: &str) -> (VaultSpreadTerms, Vec<u8>) {
    let vectors: Vec<serde_json::Value> = serde_json::from_str(ORDER_VECTORS).unwrap();
    let vector = vectors.iter().find(|v| v["name"] == name).unwrap();
    let a = &vector["args"];
    let n = |field: &str| a[field].as_str().unwrap().parse::<u64>().unwrap();
    let terms = VaultSpreadTerms {
        underlying_mint: key(3),
        quote_mint: key(4),
        expiry_ts: 1_000,
        kind: SpreadType::try_from(u8::try_from(a["position_type"].as_u64().unwrap()).unwrap())
            .unwrap(),
        side: serde_json::from_value(a["side"].clone()).unwrap(),
        lower_strike: n("strike"),
        upper_strike: n("upper_strike"),
        quantity: n("quantity"),
        price: n("gross_price"),
        valid_until: n("valid_until"),
        nonce: n("nonce"),
    };
    (
        terms,
        hex::decode(vector["preimage_hex"].as_str().unwrap()).unwrap(),
    )
}

/// The vector's order id with this sale's program, market and owners substituted.
fn rebound_order_id(
    mut preimage: Vec<u8>,
    program: &Pubkey,
    market: &Pubkey,
    maker: &Pubkey,
    taker: &Pubkey,
) -> [u8; 32] {
    for (offset, key) in [(12, program), (46, market), (110, maker), (142, taker)] {
        preimage[offset..offset + 32].copy_from_slice(key.as_ref());
    }
    Sha256::digest(&preimage).into()
}

fn signing_manager(vault_id: u8, delegate: &BytesSigner) -> VaultManager {
    VaultManager::new(VaultManagerArgs {
        delegate_authority: Pubkey::new_from_array(delegate.pubkey_bytes()),
        ..manager(vault_id).args
    })
    .unwrap()
}

fn verify_served(
    intent: &VaultOpenIntent<'_>,
    keeper: Pubkey,
    open: &VaultOpenPosition,
    order_id: [u8; 32],
    ixs: &[Instruction],
) -> Result<(), VaultIntentError> {
    let participant = match intent {
        VaultOpenIntent::Sale { writer, .. } | VaultOpenIntent::BetweenVaults { writer, .. } => {
            writer.args.actor
        }
        VaultOpenIntent::Fill { taker, .. } => *taker,
    };
    let table = lookup_table_for_open(keeper, participant, open, ixs);
    verify_served_with_tables(intent, keeper, order_id, ixs, std::slice::from_ref(&table))
}

fn verify_served_with_tables(
    intent: &VaultOpenIntent<'_>,
    keeper: Pubkey,
    order_id: [u8; 32],
    ixs: &[Instruction],
    tables: &[solana_sdk::message::AddressLookupTableAccount],
) -> Result<(), VaultIntentError> {
    let to_sign = SponsoredTxToSignData {
        order_id: OrderId::new(order_id),
        tx_base64: compile_with_tables(keeper, ixs, tables),
        last_valid_block_height: 1,
        signature_deadline: std::time::UNIX_EPOCH,
    };
    verify_vault_open(&to_sign, intent, tables, BOUNDS).map(|_| ())
}

#[test]
fn served_vault_spread_sales_verify_under_the_shared_order_layout() {
    let maker_signer = BytesSigner::from_secret([21; 32]);
    let holder = signing_manager(8, &BytesSigner::from_secret([22; 32]));
    let writer = manager(7);
    let program = writer.args.program_id;
    let maker_owner = key(9);
    let keeper = keeper();
    for (name, kind) in [
        ("spread_call", SpreadType::CallSpread),
        ("spread_put", SpreadType::PutSpread),
    ] {
        let (terms, preimage) = vector_terms(name);
        assert_eq!((terms.kind, terms.side), (kind, TakerSide::Sell));
        let market = terms.market(&program);

        let open = writer
            .open_spread_as_taker(keeper, maker_owner, &terms)
            .unwrap();
        assert_eq!(
            open.order_id,
            rebound_order_id(
                preimage.clone(),
                &program,
                &market,
                &maker_owner,
                &writer.addresses.vault
            )
        );
        let sale = VaultOpenIntent::Sale {
            keeper,
            writer: &writer,
            maker_owner,
            maker_quote_signer: Pubkey::new_from_array(maker_signer.pubkey_bytes()),
            terms: VaultSaleTerms::Spread(terms),
        };
        let served = served_sale(keeper, &open, &maker_signer);
        verify_served(&sale, keeper, &open, open.order_id, &served).unwrap();

        let between = writer
            .open_spread_with_vault_holder(keeper, &holder, &terms)
            .unwrap();
        assert_eq!(
            between.order_id,
            rebound_order_id(
                preimage,
                &program,
                &market,
                &holder.addresses.vault,
                &writer.addresses.vault
            )
        );
        let intent = VaultOpenIntent::BetweenVaults {
            keeper,
            writer: &writer,
            holder: &holder,
            terms: VaultSaleTerms::Spread(terms),
        };
        let served = served_sale(keeper, &between, &BytesSigner::from_secret([22; 32]));
        verify_served(&intent, keeper, &between, between.order_id, &served).unwrap();
        assert_eq!(
            verify_served(&intent, keeper, &between, between.order_id, &{
                let mut holder_leg = served.clone();
                holder_leg[4].accounts[10].pubkey = holder.addresses.vault_main_ata;
                holder_leg
            }),
            Err(VaultIntentError::Account {
                index: 4,
                position: 10,
                expected: holder.addresses.vault_second_ata,
                expected_signer: false,
                expected_writable: true,
                found: holder.addresses.vault_main_ata,
                found_signer: false,
                found_writable: true,
            })
        );
    }
}

#[test]
fn served_vault_spread_fills_verify_opcode_79_with_v0_alt_and_reject_tampering() {
    let holder_signer = BytesSigner::from_secret([22; 32]);
    let holder = signing_manager(8, &holder_signer);
    let keeper = keeper();
    let taker = key(9);
    let (terms, _) = vector_terms("spread_call");
    let open = holder.open_spread_as_maker(keeper, taker, &terms).unwrap();
    assert_eq!(open.instruction.data[0], 79);

    let intent = VaultOpenIntent::Fill {
        keeper,
        holder: &holder,
        taker,
        terms: VaultSaleTerms::Spread(terms),
    };
    let served = served_fill(
        keeper,
        taker,
        terms.quote_mint,
        TOKEN_PROGRAM_ID,
        &open,
        &holder_signer,
    );

    verify_served(&intent, keeper, &open, open.order_id, &served).unwrap();

    let mut tampered = served.clone();
    tampered[5].data[1] ^= 1;
    assert_eq!(
        verify_served(&intent, keeper, &open, open.order_id, &tampered),
        Err(VaultIntentError::Data {
            index: 5,
            offset: 1,
        })
    );

    let mut unsafe_table = lookup_table_for_open(keeper, taker, &open, &served);
    unsafe_table.addresses.push(open.addresses.position);
    assert_eq!(
        verify_served_with_tables(
            &intent,
            keeper,
            open.order_id,
            &served,
            std::slice::from_ref(&unsafe_table),
        ),
        Err(VaultIntentError::OpenRequiresStaticAccount(
            open.addresses.position
        ))
    );
}

#[test]
fn served_vault_open_requires_the_keeper_participant_and_dynamic_accounts_to_be_static() {
    let maker_signer = BytesSigner::from_secret([21; 32]);
    let writer = manager(7);
    let keeper = keeper();
    let maker_owner = key(9);
    let terms = terms(SpreadType::CallSpread);
    let open = writer
        .open_spread_as_taker(keeper, maker_owner, &terms)
        .unwrap();
    let intent = VaultOpenIntent::Sale {
        keeper,
        writer: &writer,
        maker_owner,
        maker_quote_signer: Pubkey::new_from_array(maker_signer.pubkey_bytes()),
        terms: VaultSaleTerms::Spread(terms),
    };
    let served = served_sale(keeper, &open, &maker_signer);

    assert!(verify_served(&intent, keeper, &open, open.order_id, &served).is_ok());
    assert!(matches!(
        verify_served(&intent, key(12), &open, open.order_id, &served),
        Err(VaultIntentError::FeePayer { .. })
    ));

    let mut wrong_keeper = served.clone();
    wrong_keeper[4].accounts[0].pubkey = key(12);
    assert!(matches!(
        verify_served(&intent, keeper, &open, open.order_id, &wrong_keeper),
        Err(VaultIntentError::Account {
            index: 4,
            position: 0,
            ..
        })
    ));

    let mut missing_participant_signer = served.clone();
    missing_participant_signer[4].accounts[1].is_signer = false;
    assert!(matches!(
        verify_served(
            &intent,
            keeper,
            &open,
            open.order_id,
            &missing_participant_signer,
        ),
        Err(VaultIntentError::Account {
            index: 4,
            position: 1,
            expected_signer: true,
            found_signer: false,
            ..
        })
    ));

    assert_eq!(
        verify_served_with_tables(&intent, keeper, open.order_id, &served, &[]),
        Err(VaultIntentError::OpenRequiresLookupTable)
    );

    let mut unsafe_table = lookup_table_for_open(keeper, writer.args.actor, &open, &served);
    unsafe_table
        .addresses
        .extend([open.addresses.position, open.addresses.collateral_ata]);
    assert_eq!(
        verify_served_with_tables(
            &intent,
            keeper,
            open.order_id,
            &served,
            std::slice::from_ref(&unsafe_table),
        ),
        Err(VaultIntentError::OpenRequiresStaticAccount(
            open.addresses.position
        ))
    );

    let mut oversized = served;
    oversized.insert(
        0,
        Instruction {
            program_id: solana_sdk::pubkey!("ComputeBudget111111111111111111111111111111"),
            accounts: Vec::new(),
            data: vec![0; 1_200],
        },
    );
    assert!(matches!(
        verify_served(&intent, keeper, &open, open.order_id, &oversized),
        Err(VaultIntentError::OpenTransactionTooLarge {
            actual,
            maximum: 1232,
        }) if actual > 1232
    ));
}

#[test]
fn served_vault_spread_sales_reject_tampered_strikes_collateral_and_accounts() {
    let maker_signer = BytesSigner::from_secret([21; 32]);
    let writer = manager(7);
    let keeper = keeper();
    let maker_owner = key(9);
    for name in ["spread_call", "spread_put"] {
        let (terms, _) = vector_terms(name);
        let open = writer
            .open_spread_as_taker(keeper, maker_owner, &terms)
            .unwrap();
        let served = served_sale(keeper, &open, &maker_signer);
        let sale = |terms| VaultOpenIntent::Sale {
            keeper,
            writer: &writer,
            maker_owner,
            maker_quote_signer: Pubkey::new_from_array(maker_signer.pubkey_bytes()),
            terms: VaultSaleTerms::Spread(terms),
        };
        let honest = sale(terms);
        let tampered = |tamper: &dyn Fn(&mut Vec<Instruction>)| {
            let mut ixs = served.clone();
            tamper(&mut ixs);
            verify_served(&honest, keeper, &open, open.order_id, &ixs)
        };

        // Kind, K1, Q (and so C), premium and K2 are each bound by the open's data.
        for offset in [1, 2, 10, 18, 74] {
            assert_eq!(
                tampered(&|ixs| ixs[4].data[offset] ^= 1),
                Err(VaultIntentError::Data { index: 4, offset }),
                "{name} byte {offset}"
            );
        }
        // Collateral escrowed in the underlying, as for a covered call, instead of the quote mint.
        let position = open.addresses.position;
        let underlying_ata = ata(&position, &terms.underlying_mint, &TOKEN_PROGRAM_ID);
        assert!(matches!(
            tampered(&|ixs| ixs[4].accounts[11].pubkey = underlying_ata),
            Err(VaultIntentError::Account { index: 4, position: 11, found, .. })
                if found == underlying_ata
        ));
        assert!(matches!(
            tampered(&|ixs| {
                ixs[2] = create_ata_idempotent(keeper, position, terms.underlying_mint, TOKEN_PROGRAM_ID);
            }),
            Err(VaultIntentError::Account { index: 2, position: 1, found, .. })
                if found == underlying_ata
        ));
        // The premium goes to someone other than the quoting maker.
        let stranger = ata(&key(10), &terms.quote_mint, &TOKEN_PROGRAM_ID);
        assert!(matches!(
            tampered(&|ixs| ixs[4].accounts[8].pubkey = stranger),
            Err(VaultIntentError::Account { index: 4, position: 8, found, .. }) if found == stranger
        ));
        assert!(matches!(
            tampered(&|ixs| ixs[4].accounts[7].pubkey = key(10)),
            Err(VaultIntentError::Account {
                index: 4,
                position: 7,
                ..
            })
        ));

        // The operator agreed to other strikes or size than the server serves.
        for agreed in [
            VaultSpreadTerms {
                lower_strike: terms.lower_strike + 1,
                ..terms
            },
            VaultSpreadTerms {
                upper_strike: terms.upper_strike + 1,
                ..terms
            },
            VaultSpreadTerms {
                quantity: terms.quantity + 1,
                ..terms
            },
        ] {
            assert_eq!(
                verify_served(&sale(agreed), keeper, &open, open.order_id, &served),
                Err(VaultIntentError::OrderId)
            );
            let rebuilt = writer
                .open_spread_as_taker(keeper, maker_owner, &agreed)
                .unwrap();
            assert_eq!(
                verify_served(&sale(agreed), keeper, &open, rebuilt.order_id, &served),
                Err(VaultIntentError::QuoteSignature("binds a different order"))
            );
        }
        assert_eq!(
            verify_served(
                &sale(VaultSpreadTerms {
                    side: TakerSide::Buy,
                    ..terms
                }),
                keeper,
                &open,
                open.order_id,
                &served
            ),
            Err(VaultIntentError::Intent(VaultChainError::InvalidInput(
                "taker_buy_unsupported"
            )))
        );
    }
}
