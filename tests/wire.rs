use acta_maker_sdk::wire::{decode_base58_32, decode_hex_32, encode_base58, encode_hex};
use proptest::prelude::*;

#[test]
fn hex_roundtrip() {
    let bytes = [7u8; 32];
    let hex = encode_hex(&bytes);
    let parsed = decode_hex_32(&hex).unwrap();
    assert_eq!(bytes, parsed);

    let with_prefix = format!("0x{hex}");
    let parsed_prefixed = decode_hex_32(&with_prefix).unwrap();
    assert_eq!(bytes, parsed_prefixed);
}

#[test]
fn base58_roundtrip() {
    let bytes = [1u8; 32];
    let b58 = encode_base58(&bytes);
    let parsed = decode_base58_32(&b58).unwrap();
    assert_eq!(bytes, parsed);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn hex_roundtrip_prop(bytes in proptest::array::uniform32(any::<u8>())) {
        let hex = encode_hex(&bytes);
        let parsed = decode_hex_32(&hex).unwrap();
        prop_assert_eq!(bytes, parsed);

        let prefixed = format!("0x{hex}");
        let parsed_prefixed = decode_hex_32(&prefixed).unwrap();
        prop_assert_eq!(bytes, parsed_prefixed);
    }

    #[test]
    fn base58_roundtrip_prop(bytes in proptest::array::uniform32(any::<u8>())) {
        let b58 = encode_base58(&bytes);
        let parsed = decode_base58_32(&b58).unwrap();
        prop_assert_eq!(bytes, parsed);
    }
}
