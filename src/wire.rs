use thiserror::Error;

/// Decodes a nullable field while requiring its JSON key to be present.
pub fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer)
}

/// Reads a `u64` sent as a canonical JSON decimal string: ASCII digits, no leading zero.
pub fn deserialize_decimal_u64<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    let text = <String as serde::Deserialize>::deserialize(deserializer)?;
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(serde::de::Error::custom(format!(
            "expected a decimal u64 string, got {text:?}"
        )));
    }
    text.parse().map_err(serde::de::Error::custom)
}

#[derive(Debug, Error)]
pub enum WireError {
    #[error("invalid hex: {0}")]
    InvalidHex(String),
    #[error("invalid base58: {0}")]
    InvalidBase58(String),
    #[error("invalid length: expected {expected}, got {actual}")]
    InvalidLength { expected: usize, actual: usize },
}

pub(crate) fn strip_0x(s: &str) -> &str {
    match s.strip_prefix("0x") {
        Some(rest) => rest,
        None => s,
    }
}

pub fn decode_hex_32(hex_str: &str) -> Result<[u8; 32], WireError> {
    let trimmed = strip_0x(hex_str);
    let bytes = hex::decode(trimmed).map_err(|err| WireError::InvalidHex(err.to_string()))?;
    if bytes.len() != 32 {
        return Err(WireError::InvalidLength {
            expected: 32,
            actual: bytes.len(),
        });
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn decode_base58_32(value: &str) -> Result<[u8; 32], WireError> {
    decode_base58_fixed::<32>(value)
}

pub fn decode_base58_64(value: &str) -> Result<[u8; 64], WireError> {
    decode_base58_fixed::<64>(value)
}

pub fn encode_base58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

fn decode_base58_fixed<const N: usize>(value: &str) -> Result<[u8; N], WireError> {
    let bytes = bs58::decode(value)
        .into_vec()
        .map_err(|err| WireError::InvalidBase58(err.to_string()))?;
    if bytes.len() != N {
        return Err(WireError::InvalidLength {
            expected: N,
            actual: bytes.len(),
        });
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}
