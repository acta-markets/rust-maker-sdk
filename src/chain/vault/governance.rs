use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::manager::VaultIx;
use super::{VaultChainError, VaultManager, pda};
use crate::VaultDelegatePermissions;
use crate::chain::ix::SYSTEM_PROGRAM_ID;

const CAPABILITIES_KNOWN_MASK: u32 = (1 << 3) - 1;
const MAX_REBALANCE_DRIFT_BPS: u16 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultPolicyUpdate {
    pub max_open_positions: u16,
    pub max_rebalance_drift_bps: u16,
    pub min_main_liquidity_bps: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultGovernanceUpdate {
    CapLimit(u64),
    Fees {
        performance_bps: u16,
        management_bps: u16,
    },
    /// Only Acta cold may propose or cancel it.
    HwmReset(u128),
    Capabilities {
        value: u32,
        mask: u32,
    },
    Policy(VaultPolicyUpdate),
    GovernanceAuthority(Pubkey),
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultGovernanceUpdateKind {
    CapLimit = 0,
    Fees = 1,
    HwmReset = 2,
    Capabilities = 3,
    Policy = 4,
    GovernanceAuthority = 5,
}

impl VaultGovernanceUpdate {
    #[must_use]
    pub const fn kind(self) -> VaultGovernanceUpdateKind {
        match self {
            Self::CapLimit(_) => VaultGovernanceUpdateKind::CapLimit,
            Self::Fees { .. } => VaultGovernanceUpdateKind::Fees,
            Self::HwmReset(_) => VaultGovernanceUpdateKind::HwmReset,
            Self::Capabilities { .. } => VaultGovernanceUpdateKind::Capabilities,
            Self::Policy(_) => VaultGovernanceUpdateKind::Policy,
            Self::GovernanceAuthority(_) => VaultGovernanceUpdateKind::GovernanceAuthority,
        }
    }

    pub fn payload(self) -> Result<[u8; 32], VaultChainError> {
        let mut payload = [0u8; 32];
        match self {
            Self::CapLimit(value) => payload[..8].copy_from_slice(&value.to_le_bytes()),
            Self::HwmReset(value) => payload[..16].copy_from_slice(&value.to_le_bytes()),
            Self::Fees {
                performance_bps,
                management_bps,
            } => {
                if performance_bps > 5_000 || management_bps > 1_000 {
                    return Err(VaultChainError::InvalidInput(
                        "fees exceed 5000 bps performance or 1000 bps management",
                    ));
                }
                payload[..2].copy_from_slice(&performance_bps.to_le_bytes());
                payload[2..4].copy_from_slice(&management_bps.to_le_bytes());
            }
            Self::Capabilities { value, mask } => {
                if mask == 0 || value & !mask != 0 || mask & !CAPABILITIES_KNOWN_MASK != 0 {
                    return Err(VaultChainError::InvalidInput(
                        "capability mask must be non-zero and contain every value bit",
                    ));
                }
                payload[..4].copy_from_slice(&value.to_le_bytes());
                payload[4..8].copy_from_slice(&mask.to_le_bytes());
            }
            Self::Policy(policy) => {
                if policy.max_open_positions == 0
                    || policy.max_rebalance_drift_bps == 0
                    || policy.max_rebalance_drift_bps > MAX_REBALANCE_DRIFT_BPS
                    || policy.min_main_liquidity_bps > 10_000
                {
                    return Err(VaultChainError::InvalidInput(
                        "vault policy requires positions > 0, drift in 1..=200 bps and liquidity <= 10000 bps",
                    ));
                }
                payload[..2].copy_from_slice(&policy.max_open_positions.to_le_bytes());
                payload[2..4].copy_from_slice(&policy.max_rebalance_drift_bps.to_le_bytes());
                payload[4..6].copy_from_slice(&policy.min_main_liquidity_bps.to_le_bytes());
            }
            Self::GovernanceAuthority(key) => {
                if key == Pubkey::default() {
                    return Err(VaultChainError::InvalidInput(
                        "vault authority update cannot use the zero pubkey",
                    ));
                }
                payload.copy_from_slice(key.as_ref());
            }
        }
        Ok(payload)
    }
}

impl VaultManager {
    pub fn propose_update(
        &self,
        update: VaultGovernanceUpdate,
    ) -> Result<Instruction, VaultChainError> {
        let kind = update.kind();
        let mut args = [0u8; 33];
        args[0] = kind as u8;
        args[1..].copy_from_slice(&update.payload()?);
        Ok(self.instruction(
            VaultIx::ProposeVaultUpdate,
            vec![
                AccountMeta::new(self.args.actor, true),
                AccountMeta::new(self.addresses.vault, false),
                AccountMeta::new(self.pending_update(kind), false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(self.addresses.global_config, false),
            ],
            &args,
        ))
    }

    pub fn apply_update(
        &self,
        kind: VaultGovernanceUpdateKind,
        proposal_payer: Pubkey,
        prices: Option<(Pubkey, Pubkey)>,
    ) -> Result<Instruction, VaultChainError> {
        let mut ix = self.instruction(
            VaultIx::ApplyVaultUpdate,
            vec![
                AccountMeta::new(self.addresses.vault, false),
                AccountMeta::new(self.addresses.policy, false),
                AccountMeta::new(self.pending_update(kind), false),
                AccountMeta::new(proposal_payer, false),
            ],
            &[kind as u8],
        );
        if matches!(
            kind,
            VaultGovernanceUpdateKind::Fees | VaultGovernanceUpdateKind::HwmReset
        ) {
            let Some((main, second)) = prices else {
                return Err(VaultChainError::InvalidInput(
                    "fee and HWM updates require both price updates to settle the old terms",
                ));
            };
            let mut equity = self.process_equity(main, second).accounts;
            equity.drain(1..4);
            ix.accounts.extend(equity);
        }
        Ok(ix)
    }

    #[must_use]
    pub fn cancel_update(
        &self,
        kind: VaultGovernanceUpdateKind,
        proposal_payer: Pubkey,
    ) -> Instruction {
        self.instruction(
            VaultIx::CancelVaultUpdate,
            vec![
                AccountMeta::new_readonly(self.args.actor, true),
                AccountMeta::new_readonly(self.addresses.global_config, false),
                AccountMeta::new(self.addresses.vault, false),
                AccountMeta::new(self.pending_update(kind), false),
                AccountMeta::new(proposal_payer, false),
            ],
            &[kind as u8],
        )
    }

    pub fn rotate_delegate(
        &self,
        new_delegate: Pubkey,
        permissions: VaultDelegatePermissions,
        expires_at: u64,
    ) -> Result<Instruction, VaultChainError> {
        let bits = permissions.bits();
        if new_delegate == Pubkey::default()
            || bits == 0
            || bits & !VaultDelegatePermissions::KNOWN_MASK != 0
        {
            return Err(VaultChainError::InvalidInput(
                "new delegate must be non-zero and have known permissions",
            ));
        }
        let vault = &self.addresses.vault;
        let new_account = pda::vault_delegate(&self.args.program_id, vault, &new_delegate).0;
        let mut args = Vec::with_capacity(44);
        args.extend_from_slice(new_delegate.as_ref());
        args.extend_from_slice(&bits.to_le_bytes());
        args.extend_from_slice(&expires_at.to_le_bytes());
        Ok(self.instruction(
            VaultIx::RotateVaultDelegate,
            vec![
                AccountMeta::new(self.args.actor, true),
                AccountMeta::new(*vault, false),
                AccountMeta::new(self.addresses.delegate, false),
                AccountMeta::new(new_account, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
            &args,
        ))
    }

    #[must_use]
    pub fn disable_delegate(&self) -> Instruction {
        self.instruction(
            VaultIx::DisableVaultDelegate,
            vec![
                AccountMeta::new_readonly(self.args.actor, true),
                AccountMeta::new_readonly(self.addresses.vault, false),
                AccountMeta::new(self.addresses.delegate, false),
            ],
            &[],
        )
    }

    fn pending_update(&self, kind: VaultGovernanceUpdateKind) -> Pubkey {
        pda::pending_vault_update(&self.args.program_id, &self.addresses.vault, kind as u8).0
    }
}
