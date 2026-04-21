use acta_maker_sdk::orders::*;
use acta_maker_sdk::wire::{encode_base58, encode_hex};
use proptest::prelude::*;

#[test]
fn order_preimage_layout_is_stable() {
    let args = OrderPreimageArgs {
        chain_id: 0,
        program_id: [4u8; 32],
        is_taker_buy: false,
        position_type: 2,
        market: [1u8; 32],
        strike: 42,
        quantity: 7,
        gross_price: 11,
        valid_until: 123,
        maker: [2u8; 32],
        taker: [3u8; 32],
        nonce: 999,
    };

    let preimage = build_order_preimage(&args);
    assert_eq!(preimage.len(), ORDER_PREIMAGE_LEN);
    assert_eq!(&preimage[0..4], &ORDER_DOMAIN_TAG);
    assert_eq!(u64::from_le_bytes(preimage[4..12].try_into().unwrap()), 0);
    assert_eq!(&preimage[12..44], &args.program_id);

    let base = 44usize;
    assert_eq!(preimage[base], 0);
    assert_eq!(preimage[base + 1], 2);
    assert_eq!(&preimage[base + 2..base + 34], &args.market);
    assert_eq!(
        u64::from_le_bytes(preimage[base + 34..base + 42].try_into().unwrap()),
        42
    );
    assert_eq!(
        u64::from_le_bytes(preimage[base + 42..base + 50].try_into().unwrap()),
        7
    );
    assert_eq!(
        u64::from_le_bytes(preimage[base + 50..base + 58].try_into().unwrap()),
        11
    );
    assert_eq!(
        u64::from_le_bytes(preimage[base + 58..base + 66].try_into().unwrap()),
        123
    );
    assert_eq!(&preimage[base + 66..base + 98], &args.maker);
    assert_eq!(&preimage[base + 98..base + 130], &args.taker);
    assert_eq!(
        u64::from_le_bytes(preimage[base + 130..base + 138].try_into().unwrap()),
        999
    );
}

#[test]
fn order_id_changes_when_nonce_changes() {
    let base = OrderPreimageArgs {
        chain_id: 0,
        program_id: [4u8; 32],
        is_taker_buy: false,
        position_type: 1,
        market: [9u8; 32],
        strike: 1_000_000_000,
        quantity: 10_000_000,
        gross_price: 2_000_000_000,
        valid_until: 555,
        maker: [7u8; 32],
        taker: [5u8; 32],
        nonce: 1,
    };
    let alt = OrderPreimageArgs {
        nonce: 2,
        ..base.clone()
    };

    let h1 = compute_order_id(&base);
    let h2 = compute_order_id(&alt);
    assert_ne!(h1, h2);
}

#[test]
fn sign_and_verify_order_id() {
    let args = OrderPreimageArgs {
        chain_id: 0,
        program_id: [1u8; 32],
        is_taker_buy: false,
        position_type: 0,
        market: [2u8; 32],
        strike: 1,
        quantity: 1,
        gross_price: 1,
        valid_until: 1,
        maker: [3u8; 32],
        taker: [4u8; 32],
        nonce: 42,
    };
    let order_id = compute_order_id(&args);

    let signing_key = [9u8; 32];
    let signature = sign_order_id_bytes(&order_id, &signing_key).unwrap();

    let verifying_key = ed25519_dalek::SigningKey::from_bytes(&signing_key).verifying_key();
    verify_order_id_signature_bytes(&order_id, &signature, &verifying_key.to_bytes()).unwrap();

    let order_id_hex = encode_hex(&order_id);
    let sig_b58 = encode_base58(&signature);
    let pubkey_b58 = encode_base58(&verifying_key.to_bytes());
    verify_order_id_signature_base58(&order_id_hex, &sig_b58, &pubkey_b58).unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn order_preimage_layout_prop(
        chain_id in any::<u64>(),
        program_id in proptest::array::uniform32(any::<u8>()),
        is_taker_buy in any::<bool>(),
        position_type in 0u8..=1,
        market in proptest::array::uniform32(any::<u8>()),
        strike in any::<u64>(),
        quantity in any::<u64>(),
        gross_price in any::<u64>(),
        valid_until in any::<u64>(),
        maker in proptest::array::uniform32(any::<u8>()),
        taker in proptest::array::uniform32(any::<u8>()),
        nonce in any::<u64>(),
    ) {
        let args = OrderPreimageArgs {
            chain_id,
            program_id,
            is_taker_buy,
            position_type,
            market,
            strike,
            quantity,
            gross_price,
            valid_until,
            maker,
            taker,
            nonce,
        };

        let preimage = build_order_preimage(&args);
        prop_assert_eq!(preimage.len(), ORDER_PREIMAGE_LEN);
        prop_assert_eq!(&preimage[0..4], &ORDER_DOMAIN_TAG);
        prop_assert_eq!(u64::from_le_bytes(preimage[4..12].try_into().unwrap()), chain_id);

        let base = 44usize;
        prop_assert_eq!(preimage[base], if is_taker_buy { 1 } else { 0 });
        prop_assert_eq!(preimage[base + 1], position_type);
        prop_assert_eq!(&preimage[base + 2..base + 34], &args.market);
        prop_assert_eq!(
            u64::from_le_bytes(preimage[base + 34..base + 42].try_into().unwrap()),
            strike
        );
        prop_assert_eq!(
            u64::from_le_bytes(preimage[base + 42..base + 50].try_into().unwrap()),
            quantity
        );
        prop_assert_eq!(
            u64::from_le_bytes(preimage[base + 50..base + 58].try_into().unwrap()),
            gross_price
        );
        prop_assert_eq!(
            u64::from_le_bytes(preimage[base + 58..base + 66].try_into().unwrap()),
            valid_until
        );
        prop_assert_eq!(&preimage[base + 66..base + 98], &args.maker);
        prop_assert_eq!(&preimage[base + 98..base + 130], &args.taker);
        prop_assert_eq!(
            u64::from_le_bytes(preimage[base + 130..base + 138].try_into().unwrap()),
            nonce
        );
    }

    #[test]
    fn order_id_changes_when_nonce_changes_prop(
        chain_id in any::<u64>(),
        program_id in proptest::array::uniform32(any::<u8>()),
        is_taker_buy in any::<bool>(),
        position_type in 0u8..=1,
        market in proptest::array::uniform32(any::<u8>()),
        strike in any::<u64>(),
        quantity in any::<u64>(),
        gross_price in any::<u64>(),
        valid_until in any::<u64>(),
        maker in proptest::array::uniform32(any::<u8>()),
        taker in proptest::array::uniform32(any::<u8>()),
        nonce in 0u64..(u64::MAX - 1),
    ) {
        let base = OrderPreimageArgs {
            chain_id,
            program_id,
            is_taker_buy,
            position_type,
            market,
            strike,
            quantity,
            gross_price,
            valid_until,
            maker,
            taker,
            nonce,
        };
        let alt = OrderPreimageArgs {
            nonce: nonce + 1,
            ..base.clone()
        };

        let h1 = compute_order_id(&base);
        let h2 = compute_order_id(&alt);
        prop_assert_ne!(h1, h2);
    }
}
