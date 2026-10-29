//! Chain-independent 32-byte account address.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address32(pub [u8; 32]);

impl Address32 {
    pub const ZERO: Self = Self([0; 32]);

    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Parse the base58 representation used by Solana account addresses.
    ///
    /// # Errors
    /// Returns an error when the value is not canonical 32-byte base58.
    pub fn from_base58(value: &str) -> Result<Self, AddressError> {
        let bytes = bs58::decode(value)
            .into_vec()
            .map_err(|_| AddressError::InvalidBase58)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|bytes: Vec<u8>| AddressError::InvalidLength(bytes.len()))?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn to_base58(self) -> String {
        bs58::encode(self.0).into_string()
    }
}

impl fmt::Debug for Address32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Address32")
            .field(&self.to_base58())
            .finish()
    }
}

impl fmt::Display for Address32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_base58())
    }
}

impl Serialize for Address32 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_base58())
    }
}

impl<'de> Deserialize<'de> for Address32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_base58(&value).map_err(serde::de::Error::custom)
    }
}

impl AsRef<[u8]> for Address32 {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8; 32]> for Address32 {
    fn as_ref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for Address32 {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<Address32> for [u8; 32] {
    fn from(address: Address32) -> Self {
        address.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("invalid base58 address")]
    InvalidBase58,
    #[error("invalid address length: expected 32 bytes, got {0}")]
    InvalidLength(usize),
}

impl From<solana_pubkey::Pubkey> for Address32 {
    fn from(value: solana_pubkey::Pubkey) -> Self {
        Self(value.to_bytes())
    }
}
impl From<Address32> for solana_pubkey::Pubkey {
    fn from(value: Address32) -> Self {
        Self::new_from_array(value.0)
    }
}
