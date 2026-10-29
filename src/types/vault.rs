use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VaultOperatorKind {
    Governance,
    Delegate,
}

/// Staging half: once `Stage` confirms, requesting the same swap again returns the `Unstage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VaultStagingStep {
    Stage,
    Unstage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VaultLeg {
    Main,
    Second,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VaultSwapMode {
    Rebalance,
    Recovery,
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VaultSwapDirection {
    MainToSecond,
    SecondToMain,
}

/// Wire permission integer; preserves unknown bits for forward compatibility.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VaultDelegatePermissions(u32);

impl VaultDelegatePermissions {
    pub const KNOWN_MASK: u32 = 0b11_1111;
    pub const OPEN_TAKER: Self = Self(1 << 0);
    pub const SIGN_MAKER_QUOTES: Self = Self(1 << 1);
    pub const FUND_MAKER: Self = Self(1 << 2);
    pub const REBALANCE: Self = Self(1 << 3);
    pub const PROCESS_CAPITAL: Self = Self(1 << 4);
    pub const SET_DEPOSIT_WINDOW: Self = Self(1 << 5);
    pub const ALL: Self = Self(Self::KNOWN_MASK);

    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// May sign quotes on the vault's behalf (`SIGN_MAKER_QUOTES`).
    #[must_use]
    pub const fn can_maker(self) -> bool {
        self.contains(Self::SIGN_MAKER_QUOTES)
    }

    /// May open taker positions for the vault (`OPEN_TAKER`).
    #[must_use]
    pub const fn can_taker(self) -> bool {
        self.contains(Self::OPEN_TAKER)
    }
}

#[cfg(test)]
mod tests {
    use super::VaultSwapMode;

    #[test]
    fn closing_mode_is_a_known_wire_value() {
        let mode: VaultSwapMode = serde_json::from_str("\"closing\"").unwrap();
        assert_eq!(mode, VaultSwapMode::Closing);
        assert_eq!(serde_json::to_string(&mode).unwrap(), "\"closing\"");
        assert_eq!(<&str>::from(mode), "closing");
    }
}
