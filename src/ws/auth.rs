use crate::SigningError;

const AUTH_PREFIX: &str =
    "Acta RFQ Authentication\n\nSign this message to authenticate your wallet.\n\n";

pub fn validate_maker_auth_challenge(challenge: &str) -> Result<(), SigningError> {
    if challenge
        .strip_prefix(AUTH_PREFIX)
        .is_some_and(valid_nonce_and_time)
    {
        Ok(())
    } else {
        Err(SigningError::new("invalid maker authentication challenge"))
    }
}

/// Validate before handing the challenge to any signer.
pub fn validate_vault_auth_challenge(
    challenge: &str,
    operator: &str,
    vault: &str,
) -> Result<(), SigningError> {
    let invalid = || SigningError::new("invalid vault authentication challenge");
    crate::wire::decode_base58_32(operator).map_err(|_| invalid())?;
    crate::wire::decode_base58_32(vault).map_err(|_| invalid())?;
    let body = challenge
        .strip_prefix(AUTH_PREFIX)
        .and_then(|s| s.strip_prefix("Wallet: "))
        .and_then(|s| s.strip_prefix(operator))
        .and_then(|s| s.strip_prefix('\n'))
        .and_then(|s| s.strip_suffix(vault))
        .and_then(|s| s.strip_suffix("\nVault: "))
        .ok_or_else(invalid)?;
    if !valid_nonce_and_time(body) {
        return Err(invalid());
    }
    Ok(())
}

fn valid_nonce_and_time(body: &str) -> bool {
    let Some((nonce, timestamp)) = body
        .strip_prefix("Nonce: ")
        .and_then(|s| s.split_once("\nIssued At: "))
    else {
        return false;
    };
    nonce.len() == 64
        && nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && timestamp.strip_suffix('\n').is_some_and(is_utc_seconds)
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
pub(crate) fn test_vault_auth_challenge(operator: &str, vault: &str) -> String {
    let challenge =
        TEST_MAKER_AUTH_CHALLENGE.replace("Nonce: ", &format!("Wallet: {operator}\nNonce: "));
    format!("{challenge}\nVault: {vault}")
}

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
