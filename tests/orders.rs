use acta_maker_sdk::PositionType;
use acta_maker_sdk::orders::*;
use acta_maker_sdk::wire::{encode_base58, encode_hex};
use proptest::prelude::*;

#[test]
fn order_preimage_layout_is_stable() {
    let args = OrderPreimageArgs {
        chain_id: 0,
        program_id: [4u8; 32],
        is_taker_buy: false,
        position_type: PositionType::CashSecuredPut,
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
    assert_eq!(preimage[base + 1], 1);
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
        position_type: PositionType::CashSecuredPut,
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
        position_type: PositionType::CoveredCall,
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

#[test]
fn bytes_signer_from_keypair_validates_and_signs() {
    let mut keypair = [0u8; 64];
    keypair[0..32].copy_from_slice(&[9u8; 32]);
    let public = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
        .verifying_key()
        .to_bytes();
    keypair[32..64].copy_from_slice(&public);
    let signer = BytesSigner::from_keypair(&keypair).unwrap();

    let order_id = [7u8; ORDER_ID_LEN];
    let signature = signer.sign_message(&order_id);
    verify_order_id_signature_bytes(&order_id, &signature, &signer.pubkey_bytes()).unwrap();
}

#[test]
fn bytes_signer_rejects_mismatched_public_half() {
    let mut keypair = [0u8; 64];
    keypair[0..32].copy_from_slice(&[9u8; 32]);
    keypair[32..64].copy_from_slice(&[7u8; 32]);

    assert!(matches!(
        BytesSigner::from_keypair(&keypair),
        Err(OrderError::KeypairPublicKeyMismatch)
    ));
}

#[test]
fn base58_keypair_signing_rejects_mismatched_public_half() {
    let mut keypair = [0u8; 64];
    keypair[0..32].copy_from_slice(&[9u8; 32]);
    keypair[32..64].copy_from_slice(&[7u8; 32]);

    assert!(matches!(
        sign_order_id_from_base58_keypair(&encode_hex(&[1u8; 32]), &encode_base58(&keypair)),
        Err(OrderError::KeypairPublicKeyMismatch)
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn order_preimage_layout_prop(
        chain_id in any::<u64>(),
        program_id in proptest::array::uniform32(any::<u8>()),
        is_taker_buy in any::<bool>(),
        is_put in any::<bool>(),
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
            position_type: if is_put {
                PositionType::CashSecuredPut
            } else {
                PositionType::CoveredCall
            },
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
        prop_assert_eq!(preimage[base + 1], u8::from(args.position_type));
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
        is_put in any::<bool>(),
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
            position_type: if is_put {
                PositionType::CashSecuredPut
            } else {
                PositionType::CoveredCall
            },
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

#[test]
fn order_id_matches_contract_golden_vector() {
    // Same fixed input and expected bytes as program/src/utils/order_hash.rs.
    let preimage = build_order_preimage(&OrderPreimageArgs {
        chain_id: 0x0102030405060708,
        is_taker_buy: true,
        strike: 0x1112131415161718,
        quantity: 0x2122232425262728,
        gross_price: 0x3132333435363738,
        valid_until: 0x4142434445464748,
        nonce: 0x5152535455565758,
        position_type: PositionType::CashSecuredPut,
        program_id: std::array::from_fn(|i| i as u8),
        market: std::array::from_fn(|i| (i + 32) as u8),
        maker: std::array::from_fn(|i| (i + 64) as u8),
        taker: std::array::from_fn(|i| (i + 96) as u8),
    });
    let expected_preimage = [
        0x41, 0x43, 0x54, 0x41, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0x01, 0x02,
        0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11,
        0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x01,
        0x01, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d,
        0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c,
        0x3d, 0x3e, 0x3f, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x28, 0x27, 0x26, 0x25,
        0x24, 0x23, 0x22, 0x21, 0x38, 0x37, 0x36, 0x35, 0x34, 0x33, 0x32, 0x31, 0x48, 0x47, 0x46,
        0x45, 0x44, 0x43, 0x42, 0x41, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
        0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
        0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67,
        0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76,
        0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f, 0x58, 0x57, 0x56, 0x55, 0x54, 0x53,
        0x52, 0x51,
    ];
    let expected_digest = [
        0xc7, 0x0e, 0x2d, 0x1b, 0xb8, 0x9a, 0xfe, 0xcd, 0xb0, 0xc4, 0x59, 0x1d, 0x1b, 0x0d, 0x6e,
        0xdc, 0x62, 0xf8, 0xb8, 0x27, 0x7b, 0xd5, 0xcb, 0xb2, 0x0e, 0x51, 0x98, 0xe8, 0xa2, 0x04,
        0x31, 0x78,
    ];
    assert_eq!(preimage, expected_preimage);
    assert_eq!(hash_order_preimage(&preimage), expected_digest);
}
