use acta_maker_sdk::{ReferralCode, ReferralCodeFormatError, TakerStatus, is_reserved};

#[test]
fn referral_code_normalizes_the_backend_format() {
    let code = ReferralCode::parse("  abc1 ").unwrap();
    assert_eq!(code.as_str(), "ABC1");
    assert_eq!(serde_json::to_string(&code).unwrap(), "\"ABC1\"");
}

#[test]
fn referral_code_rejects_invalid_length_and_charset() {
    assert!(matches!(
        ReferralCode::parse("abc"),
        Err(ReferralCodeFormatError::Length { .. })
    ));
    assert!(matches!(
        ReferralCode::parse("ABC-1"),
        Err(ReferralCodeFormatError::Charset)
    ));
}

#[test]
fn reserved_codes_match_case_insensitively() {
    assert!(is_reserved("admin"));
    assert!(!is_reserved("MAKER1"));
}

#[test]
fn taker_status_parses_the_wire_spelling() {
    assert_eq!(
        TakerStatus::try_from("active".to_string()).unwrap(),
        TakerStatus::Active
    );
    assert!(TakerStatus::try_from("ACTIVE".to_string()).is_err());
}
