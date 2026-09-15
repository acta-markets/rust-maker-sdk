use crate::SigningError;

const MAKER_AUTH_PREFIX: &str =
    "Acta RFQ Authentication\n\nSign this message to authenticate your wallet.\n\nNonce: ";

/// Validate the maker authentication message before passing it to a signer.
///
/// Maker challenges have no wallet line: the response identifies the owner,
/// while its registered signing key may be different. This message format is
/// disjoint from the raw 32-byte order id. Freshness is checked by the server.
///
/// # Errors
/// Rejects a message outside the server's canonical maker-auth format.
pub fn validate_maker_auth_challenge(challenge: &str) -> Result<(), SigningError> {
    let invalid = || SigningError::new("invalid maker authentication challenge");
    let body = challenge
        .strip_prefix(MAKER_AUTH_PREFIX)
        .ok_or_else(invalid)?;
    let (nonce, timestamp) = body.split_once("\nIssued At: ").ok_or_else(invalid)?;
    if nonce.len() != 64
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !timestamp.strip_suffix('\n').is_some_and(is_utc_seconds)
    {
        return Err(invalid());
    }
    Ok(())
}

fn is_utc_seconds(timestamp: &str) -> bool {
    let b = timestamp.as_bytes();
    if b.len() != 20
        || !b.iter().enumerate().all(|(i, byte)| match i {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
    {
        return false;
    }
    let pair = |i: usize| (b[i] - b'0') * 10 + b[i + 1] - b'0';
    let year = u16::from(pair(0)) * 100 + u16::from(pair(2));
    let days = match pair(5) {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    (1..=days).contains(&pair(8)) && pair(11) < 24 && pair(14) < 60 && pair(17) < 60
}

#[cfg(test)]
pub(crate) const TEST_MAKER_AUTH_CHALLENGE: &str = concat!(
    "Acta RFQ Authentication\n\nSign this message to authenticate your wallet.\n\n",
    "Nonce: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
    "Issued At: 2026-09-15T00:00:00Z\n"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_maker_challenge_is_accepted() {
        validate_maker_auth_challenge(TEST_MAKER_AUTH_CHALLENGE).expect("canonical");
    }

    #[test]
    fn rejects_noncanonical_or_cross_domain_messages() {
        let canonical = TEST_MAKER_AUTH_CHALLENGE;
        for message in [
            "x".repeat(32),
            canonical.replace("Acta RFQ Authentication", "Acta Admin Authentication"),
            canonical.replace("Nonce: ", "Wallet: owner\nNonce: "),
            canonical.replace("Nonce: ", "Nonce: a"),
            canonical.replace("abcdef", "ABCDEF"),
            canonical.replace('\n', "\r\n"),
            canonical.replace("2026-09-15", "2026-02-30"),
            canonical.replace("00:00:00Z", "00:00:00.000Z"),
            canonical.replace("00:00:00Z", "24:00:00Z"),
            canonical.replace("00:00:00Z", "00:00:00+00:00"),
            canonical.trim_end().to_owned(),
            format!("{canonical}extra\n"),
        ] {
            assert!(
                validate_maker_auth_challenge(&message).is_err(),
                "{message:?}"
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn ascii_order_sized_messages_are_rejected(bytes in proptest::array::uniform32(0u8..=127)) {
            let message = std::str::from_utf8(&bytes).expect("ASCII is UTF-8");
            proptest::prop_assert!(validate_maker_auth_challenge(message).is_err());
        }
    }
}
