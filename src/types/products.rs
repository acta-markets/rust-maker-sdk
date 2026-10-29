use crate::types::PositionType;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Explicit non-empty product policy. Missing policy is never an all-products policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductSet(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("products must be a non-empty set of supported products")]
pub struct InvalidProductSet;

const fn product_bit(product: PositionType) -> u8 {
    match product {
        PositionType::CoveredCall => 1,
        PositionType::CashSecuredPut => 2,
        PositionType::CallSpread => 4,
        PositionType::PutSpread => 8,
        PositionType::CashPut => 16,
    }
}

impl ProductSet {
    pub const ALL: Self = Self(31);
    pub const PHYSICAL: Self = Self(3);
    pub fn new(products: &[PositionType]) -> Result<Self, InvalidProductSet> {
        let mut bits = 0u8;
        for product in products {
            let bit = product_bit(*product);
            if bits & bit != 0 {
                return Err(InvalidProductSet);
            }
            bits |= bit;
        }
        Self::from_bits(bits)
    }

    pub const fn from_bits(bits: u8) -> Result<Self, InvalidProductSet> {
        if bits == 0 || bits > 31 {
            Err(InvalidProductSet)
        } else {
            Ok(Self(bits))
        }
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub fn contains(self, product: PositionType) -> bool {
        self.0 & (product_bit(product)) != 0
    }

    pub fn iter(self) -> impl Iterator<Item = PositionType> {
        [
            PositionType::CoveredCall,
            PositionType::CashSecuredPut,
            PositionType::CallSpread,
            PositionType::PutSpread,
            PositionType::CashPut,
        ]
        .into_iter()
        .filter(move |product| self.contains(*product))
    }
}

impl Serialize for ProductSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.iter().collect::<Vec<_>>().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProductSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let products = Vec::<PositionType>::deserialize(deserializer)?;
        Self::new(&products).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cash_put_has_one_explicit_strike_and_put_spread_execution() {
        let policy: ProductSet = serde_json::from_str(r#"["cash_put"]"#).unwrap();
        assert!(policy.contains(PositionType::CashPut));
        assert_eq!(serde_json::to_string(&policy).unwrap(), r#"["cash_put"]"#);
        let terms: crate::types::OptionTerms =
            serde_json::from_str(r#"{"type":"cash_put","strike":100}"#).unwrap();
        assert_eq!(terms.position_type(), PositionType::CashPut);
        assert_eq!(u8::from(terms.to_chain().position_type()), 3);
        assert_eq!(terms.strike().value(), 100);
        assert_eq!(terms.to_chain().strike().value(), 0);
        assert_eq!(terms.upper_strike().unwrap().value(), 100);
        assert_eq!(
            serde_json::to_string(&terms).unwrap(),
            r#"{"type":"cash_put","strike":100}"#
        );
        assert!(
            serde_json::from_str::<crate::types::OptionTerms>(
                r#"{"type":"cash_put","strike":100,"lower_strike":0}"#
            )
            .is_err()
        );
    }

    #[test]
    fn policy_requires_a_nonempty_unique_product_set() {
        for json in ["[]", r#"["call_spread","call_spread"]"#, r#"["unknown"]"#] {
            assert!(serde_json::from_str::<ProductSet>(json).is_err());
        }
        let policy: ProductSet = serde_json::from_str(r#"["put_spread","covered_call"]"#).unwrap();
        assert!(policy.contains(PositionType::PutSpread));
        assert!(!policy.contains(PositionType::CallSpread));
        assert_eq!(
            serde_json::to_string(&policy).unwrap(),
            r#"["covered_call","put_spread"]"#
        );
    }
}
