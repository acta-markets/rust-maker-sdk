use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::orders::SignerLike;

const SIGNATURE_LEN: usize = 64;
const PUBKEY_LEN: usize = 32;
const HEADER_LEN: usize = 3;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultTxError {
    #[error("transaction is not valid base64")]
    InvalidBase64,
    #[error("transaction differs from the locally approved message")]
    MessageMismatch,
    #[error("transaction bytes are malformed: {0}")]
    Malformed(&'static str),
    #[error("unsupported transaction message version {0}")]
    UnsupportedVersion(u8),
    #[error("signer {signer} is not one of the transaction's required signers")]
    SignerNotRequired { signer: String },
    #[error("{field} `{deadline}` is not a unix timestamp")]
    InvalidDeadline {
        field: &'static str,
        deadline: String,
    },
    #[error("{field} has passed: deadline {deadline}, now {now}")]
    Expired {
        field: &'static str,
        deadline: u64,
        now: u64,
    },
    #[error("system clock is before the unix epoch")]
    ClockBeforeEpoch,
    #[cfg(feature = "chain")]
    #[error(transparent)]
    Intent(#[from] crate::chain::vault::VaultIntentError),
}

/// UNVERIFIED: fills the signer's slot without checking intent; other slots stay untouched.
pub fn sign_transaction_base64_unverified(
    tx_base64: &str,
    signer: &dyn SignerLike,
) -> Result<String, VaultTxError> {
    Ok(SigningTransaction::parse(tx_base64, signer.pubkey_bytes())?.sign(signer))
}

/// Signs only if the message equals `expected_message`, built locally rather than copied from the server.
pub fn sign_transaction_base64(
    tx_base64: &str,
    expected_message: &[u8],
    signer: &dyn SignerLike,
) -> Result<String, VaultTxError> {
    Ok(verify_transaction_message(tx_base64, expected_message, signer)?.sign(signer))
}

pub(super) fn verify_transaction_message(
    tx_base64: &str,
    expected_message: &[u8],
    signer: &dyn SignerLike,
) -> Result<SigningTransaction, VaultTxError> {
    let transaction = SigningTransaction::parse(tx_base64, signer.pubkey_bytes())?;
    if transaction.bytes[transaction.message_start..] != *expected_message {
        return Err(VaultTxError::MessageMismatch);
    }
    Ok(transaction)
}

pub(super) struct SigningTransaction {
    bytes: Vec<u8>,
    signatures_start: usize,
    message_start: usize,
    signer_index: usize,
}

impl SigningTransaction {
    pub(super) fn sign(mut self, signer: &dyn SignerLike) -> String {
        let signature = signer.sign_message(&self.bytes[self.message_start..]);
        let slot = self.signatures_start + self.signer_index * SIGNATURE_LEN;
        self.bytes[slot..slot + SIGNATURE_LEN].copy_from_slice(&signature);
        STANDARD.encode(self.bytes)
    }

    fn parse(tx_base64: &str, signer: [u8; PUBKEY_LEN]) -> Result<Self, VaultTxError> {
        let bytes = STANDARD
            .decode(tx_base64)
            .map_err(|_| VaultTxError::InvalidBase64)?;
        let mut pos = 0;
        let signature_count = read_compact_u16(&bytes, &mut pos)?;
        let signatures_start = pos;
        let message_start = signatures_start
            .checked_add(signature_count.saturating_mul(SIGNATURE_LEN))
            .filter(|start| *start < bytes.len())
            .ok_or(VaultTxError::Malformed("signatures exceed transaction"))?;
        let message = &bytes[message_start..];

        let mut mpos = 0;
        if let Some(first) = message.first().filter(|first| *first & 0x80 != 0) {
            let version = first & 0x7f;
            if version != 0 {
                return Err(VaultTxError::UnsupportedVersion(version));
            }
            mpos = 1;
        }
        let required = message
            .get(mpos)
            .map(|count| usize::from(*count))
            .ok_or(VaultTxError::Malformed("missing message header"))?;
        mpos = mpos
            .checked_add(HEADER_LEN)
            .ok_or(VaultTxError::Malformed("missing message header"))?;
        let key_count = read_compact_u16(message, &mut mpos)?;
        if required != signature_count {
            return Err(VaultTxError::Malformed(
                "signature count does not match the message header",
            ));
        }
        if required > key_count {
            return Err(VaultTxError::Malformed(
                "more required signers than account keys",
            ));
        }
        let keys_end = mpos
            .checked_add(key_count.saturating_mul(PUBKEY_LEN))
            .filter(|end| *end <= message.len())
            .ok_or(VaultTxError::Malformed("account keys exceed message"))?;
        let signer_index = message[mpos..keys_end]
            .chunks_exact(PUBKEY_LEN)
            .take(required)
            .position(|key| key == signer)
            .ok_or_else(|| VaultTxError::SignerNotRequired {
                signer: crate::wire::encode_base58(&signer),
            })?;
        Ok(Self {
            bytes,
            signatures_start,
            message_start,
            signer_index,
        })
    }
}

fn read_compact_u16(bytes: &[u8], pos: &mut usize) -> Result<usize, VaultTxError> {
    let mut value = 0usize;
    for shift in [0u32, 7, 14] {
        let byte = *bytes
            .get(*pos)
            .ok_or(VaultTxError::Malformed("truncated length prefix"))?;
        *pos += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(VaultTxError::Malformed("length prefix too long"))
}

pub fn ensure_deadline_ahead(field: &'static str, deadline: &str) -> Result<u64, VaultTxError> {
    ensure_deadline_ahead_at(field, deadline, unix_now()?)
}

pub(super) fn ensure_deadline_ahead_at(
    field: &'static str,
    deadline: &str,
    now: u64,
) -> Result<u64, VaultTxError> {
    let deadline_secs =
        deadline
            .trim()
            .parse::<u64>()
            .map_err(|_| VaultTxError::InvalidDeadline {
                field,
                deadline: deadline.to_owned(),
            })?;
    if now >= deadline_secs {
        return Err(VaultTxError::Expired {
            field,
            deadline: deadline_secs,
            now,
        });
    }
    Ok(deadline_secs)
}

pub(super) fn ensure_system_time_ahead(
    field: &'static str,
    deadline: SystemTime,
) -> Result<(), VaultTxError> {
    let deadline_secs = deadline
        .duration_since(UNIX_EPOCH)
        .map_err(|_| VaultTxError::ClockBeforeEpoch)?
        .as_secs();
    let now = unix_now()?;
    if now >= deadline_secs {
        return Err(VaultTxError::Expired {
            field,
            deadline: deadline_secs,
            now,
        });
    }
    Ok(())
}

fn unix_now() -> Result<u64, VaultTxError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| VaultTxError::ClockBeforeEpoch)
}
