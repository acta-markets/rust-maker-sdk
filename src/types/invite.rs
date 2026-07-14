use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const REFERRAL_CODE_MIN_LEN: usize = 4;
pub const REFERRAL_CODE_MAX_LEN: usize = 16;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReferralCodeFormatError {
    #[error("code length must be between {min} and {max}")]
    Length { min: usize, max: usize },
    #[error("code must contain only ASCII letters and digits")]
    Charset,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReferralCode(String);

impl ReferralCode {
    /// Parse and normalize a referral code using the backend contract.
    ///
    /// # Errors
    /// Returns [`ReferralCodeFormatError::Length`] for a trimmed length outside
    /// the supported range and [`ReferralCodeFormatError::Charset`] for any
    /// non-ASCII-alphanumeric character.
    pub fn parse(input: &str) -> Result<Self, ReferralCodeFormatError> {
        let normalized = input.trim().to_ascii_uppercase();
        if !(REFERRAL_CODE_MIN_LEN..=REFERRAL_CODE_MAX_LEN).contains(&normalized.len()) {
            return Err(ReferralCodeFormatError::Length {
                min: REFERRAL_CODE_MIN_LEN,
                max: REFERRAL_CODE_MAX_LEN,
            });
        }
        if !normalized
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(ReferralCodeFormatError::Charset);
        }
        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for ReferralCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for ReferralCode {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ReferralCode {
    type Error = ReferralCodeFormatError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl TryFrom<&str> for ReferralCode {
    type Error = ReferralCodeFormatError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl Serialize for ReferralCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ReferralCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::IntoStaticStr,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TakerStatus {
    Pending,
    Active,
}

impl TryFrom<String> for TakerStatus {
    type Error = strum::ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        std::str::FromStr::from_str(value.as_str())
    }
}

pub const RESERVED_CODES: &[&str] = &["ACTA", "ADMIN", "SYSTEM", "SUPPORT", "NULL", "ROOT", "API"];

#[must_use]
pub fn is_reserved(code: &str) -> bool {
    RESERVED_CODES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(code))
}
