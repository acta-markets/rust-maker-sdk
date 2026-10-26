use std::collections::HashSet;

use sha2::{Digest, Sha256};

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::{PriceRoute, VaultAccount, VaultChainError, VaultRequestKind, pda};
use crate::chain::ix::{
    ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
    derive_associated_token_address as ata, derive_associated_token_address_with_bump,
};

/// Users per `ProcessDeposits` / `ProcessWithdrawals`: four overflow the 1232-byte packet.
pub const VAULT_CAPITAL_BATCH_USERS: usize = 3;

const BATCH_BASE_COMPUTE_UNITS: u32 = 40_000;
const COMPUTE_UNITS_PER_USER: u32 = 13_000;
const COMPUTE_UNITS_PER_CREATED_DESTINATION: u32 = 25_000;
const COMPUTE_UNITS_PER_DERIVATION_STEP: u32 = 1_800;

#[repr(u8)]
#[derive(Clone, Copy)]
pub(super) enum VaultIx {
    Bootstrap = 64,
    TopUpManagerStake = 65,
    UnlockManagerStake = 48,
    AcceptGovernance = 58,
    RevokeGovernanceNomination = 63,
    ApplyFeeRecipientUpdate = 66,
    StartShutdown = 67,
    FinalizeShutdown = 69,
    CloseVault = 59,
    ProcessVaultEquity = 37,
    OpenPositionForVaultAsTaker = 41,
    OpenPositionForVaultAsMaker = 42,
    VaultFundMakerPosition = 43,
    VaultSpotRebalance = 45,
    RecoveryConvertToMain = 46,
    ProcessWithdrawals = 38,
    ProcessDeposits = 39,
    FinalizeVaultCycle = 40,
    SetAcceptingDeposits = 44,
    RotateVaultDelegate = 47,
    ProposeVaultUpdate = 55,
    ApplyVaultUpdate = 56,
    CancelVaultUpdate = 57,
    ProcessFinalWithdrawal = 60,
    OpenPositionBetweenVaults = 61,
    DisableVaultDelegate = 62,
    ClosingConvertToMain = 68,
    VaultStagingSwap = 76,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositPayout {
    user: Pubkey,
    refund: Option<Pubkey>,
    shares: Option<Pubkey>,
}

impl DepositPayout {
    #[must_use]
    pub const fn new(user: Pubkey) -> Self {
        Self {
            user,
            refund: None,
            shares: None,
        }
    }

    #[must_use]
    pub const fn refund_to(mut self, account: Pubkey) -> Self {
        self.refund = Some(account);
        self
    }

    #[must_use]
    pub const fn shares_to(mut self, account: Pubkey) -> Self {
        self.shares = Some(account);
        self
    }
}

impl From<Pubkey> for DepositPayout {
    fn from(user: Pubkey) -> Self {
        Self::new(user)
    }
}

impl From<&Pubkey> for DepositPayout {
    fn from(user: &Pubkey) -> Self {
        Self::new(*user)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithdrawalPayout {
    user: Pubkey,
    shares_back: Option<Pubkey>,
}

impl WithdrawalPayout {
    #[must_use]
    pub const fn new(user: Pubkey) -> Self {
        Self {
            user,
            shares_back: None,
        }
    }

    #[must_use]
    pub const fn shares_back_to(mut self, account: Pubkey) -> Self {
        self.shares_back = Some(account);
        self
    }
}

impl From<Pubkey> for WithdrawalPayout {
    fn from(user: Pubkey) -> Self {
        Self::new(user)
    }
}

impl From<&Pubkey> for WithdrawalPayout {
    fn from(user: &Pubkey) -> Self {
        Self::new(*user)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapitalBatchShape {
    pub users: u32,
    pub created_destinations: u32,
    /// Off-curve PDA search steps, including ATA creation searches.
    pub derivation_steps: u32,
}

#[must_use]
pub fn vault_capital_batch_compute_units(shape: CapitalBatchShape) -> u32 {
    let users = shape.users.min(batch_users(VAULT_CAPITAL_BATCH_USERS));
    let created = shape.created_destinations.min(users * 2);
    BATCH_BASE_COMPUTE_UNITS
        .saturating_add(users * COMPUTE_UNITS_PER_USER)
        .saturating_add(created * COMPUTE_UNITS_PER_CREATED_DESTINATION)
        .saturating_add(
            shape
                .derivation_steps
                .saturating_mul(COMPUTE_UNITS_PER_DERIVATION_STEP),
        )
}

fn batch_users(len: usize) -> u32 {
    len.min(VAULT_CAPITAL_BATCH_USERS) as u32
}

const fn steps(bump: u8) -> u32 {
    (u8::MAX - bump) as u32
}

fn ata_steps(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> u32 {
    steps(derive_associated_token_address_with_bump(owner, mint, token_program).1)
}

fn max_creation_steps(
    mut destination_steps: [u32; VAULT_CAPITAL_BATCH_USERS],
    count: usize,
) -> u32 {
    destination_steps.sort_unstable_by(|left, right| right.cmp(left));
    destination_steps
        .iter()
        .take(count.min(VAULT_CAPITAL_BATCH_USERS))
        .sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultManagerArgs {
    pub program_id: Pubkey,
    pub vault_id: [u8; 32],
    pub actor: Pubkey,
    /// Authority stored in the vault's current delegate account.
    pub delegate_authority: Pubkey,
    pub main_mint: Pubkey,
    pub second_mint: Pubkey,
    pub share_mint: Pubkey,
    pub main_token_program: Pubkey,
    pub second_token_program: Pubkey,
    /// `VaultAccount::second_leg_generation` that `second_mint` belongs to.
    pub second_leg_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultAddresses {
    pub vault: Pubkey,
    pub policy: Pubkey,
    pub delegate: Pubkey,
    pub global_config: Pubkey,
    pub swap_venues: Pubkey,
    pub main_price_route: Pubkey,
    pub second_price_route: Pubkey,
    pub manager_stake_authority: Pubkey,
    pub vault_main_ata: Pubkey,
    pub vault_second_ata: Pubkey,
    pub manager_stake_ata: Pubkey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultManager {
    pub(super) args: VaultManagerArgs,
    pub addresses: VaultAddresses,
}

impl VaultManager {
    pub fn from_vault(
        program_id: Pubkey,
        actor: Pubkey,
        vault: &VaultAccount,
    ) -> Result<Self, VaultChainError> {
        Self::new(VaultManagerArgs {
            program_id,
            vault_id: vault.vault_id,
            actor,
            delegate_authority: vault.primary_delegate,
            main_mint: vault.main_mint,
            second_mint: vault.second_mint,
            share_mint: vault.share_mint,
            main_token_program: vault.main_token_program,
            second_token_program: vault.second_token_program,
            second_leg_generation: vault.second_leg_generation,
        })
    }

    /// Errs when `vault` is another vault or its second leg rolled since this manager was built.
    pub fn ensure_current(&self, vault: &VaultAccount) -> Result<(), VaultChainError> {
        let args = &self.args;
        if vault.vault_id != args.vault_id {
            return Err(VaultChainError::InvalidInput(
                "vault account belongs to another vault",
            ));
        }
        if vault.second_leg_generation != args.second_leg_generation
            || vault.second_mint != args.second_mint
            || vault.second_token_program != args.second_token_program
        {
            return Err(VaultChainError::SecondLegRolled);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_account(&self, staging: super::VaultStagingState) -> VaultAccount {
        let args = &self.args;
        VaultAccount {
            vault_id: args.vault_id,
            phase: super::VaultPhase::ProcessingCapital,
            safety: super::VaultSafety::Normal,
            equity_computed: false,
            accepting_deposits: false,
            governance_authority: args.actor,
            primary_delegate: args.delegate_authority,
            pending_governance_authority: Pubkey::default(),
            fee_recipient: args.actor,
            main_mint: args.main_mint,
            second_mint: args.second_mint,
            share_mint: args.share_mint,
            main_token_program: args.main_token_program,
            second_token_program: args.second_token_program,
            active_expiry: 0,
            open_positions: 0,
            total_shares: 0,
            pending_deposit_count: 0,
            pending_withdraw_count: 0,
            capital_fallback_delay_secs: 0,
            closing_started_at: 0,
            boundary_equity: 0,
            boundary_shares: 0,
            staging,
            second_leg_generation: args.second_leg_generation,
        }
    }

    /// Rebuilds the second leg (mint, ATA, price route) from `vault`; returns whether it rolled.
    pub fn refresh(&mut self, vault: &VaultAccount) -> Result<bool, VaultChainError> {
        match self.ensure_current(vault) {
            Err(VaultChainError::SecondLegRolled) => {
                *self = Self::new(VaultManagerArgs {
                    second_mint: vault.second_mint,
                    second_token_program: vault.second_token_program,
                    second_leg_generation: vault.second_leg_generation,
                    ..self.args
                })?;
                Ok(true)
            }
            other => other.map(|()| false),
        }
    }

    pub fn new(args: VaultManagerArgs) -> Result<Self, VaultChainError> {
        if args.vault_id == [0; 32]
            || args.actor == Pubkey::default()
            || args.delegate_authority == Pubkey::default()
        {
            return Err(VaultChainError::InvalidInput(
                "vault id, actor and delegate authority must be non-zero",
            ));
        }
        if args.main_mint == args.second_mint
            || args.share_mint == args.main_mint
            || args.share_mint == args.second_mint
        {
            return Err(VaultChainError::InvalidInput(
                "vault main, second and share mints must be distinct",
            ));
        }
        for program in [args.main_token_program, args.second_token_program] {
            if program != TOKEN_PROGRAM_ID && program != TOKEN_2022_PROGRAM_ID {
                return Err(VaultChainError::InvalidInput(
                    "pair token program must be legacy SPL or Token-2022",
                ));
            }
        }
        let program_id = &args.program_id;
        let vault = pda::vault(program_id, &args.vault_id).0;
        let manager_stake_authority = pda::manager_stake(program_id, &vault).0;
        let addresses = VaultAddresses {
            vault,
            policy: pda::vault_policy(program_id, &vault).0,
            delegate: pda::vault_delegate(program_id, &vault, &args.delegate_authority).0,
            global_config: pda::global_config(program_id).0,
            swap_venues: pda::swap_venues(program_id).0,
            main_price_route: pda::price_route(program_id, &args.main_mint).0,
            second_price_route: pda::price_route(program_id, &args.second_mint).0,
            manager_stake_authority,
            vault_main_ata: ata(&vault, &args.main_mint, &args.main_token_program),
            vault_second_ata: ata(&vault, &args.second_mint, &args.second_token_program),
            manager_stake_ata: ata(
                &manager_stake_authority,
                &args.share_mint,
                &TOKEN_PROGRAM_ID,
            ),
        };
        Ok(Self { args, addresses })
    }

    #[must_use]
    pub fn set_accepting_deposits(&self, accepting: bool) -> Instruction {
        self.instruction(
            VaultIx::SetAcceptingDeposits,
            vec![
                AccountMeta::new_readonly(self.args.actor, true),
                AccountMeta::new(self.addresses.vault, false),
                AccountMeta::new_readonly(self.addresses.delegate, false),
                AccountMeta::new_readonly(self.args.share_mint, false),
                AccountMeta::new_readonly(self.addresses.manager_stake_ata, false),
            ],
            &[u8::from(accepting)],
        )
    }

    /// Each leg's route account, then its bricks' accounts, Pyth read on `shard`.
    pub fn price_section(
        &self,
        main: &PriceRoute,
        second: &PriceRoute,
        shard: u16,
    ) -> Result<Vec<AccountMeta>, VaultChainError> {
        if main.mint != self.args.main_mint || second.mint != self.args.second_mint {
            return Err(VaultChainError::InvalidInput(
                "price routes must price the vault's main and second mints",
            ));
        }
        let mut section = main.accounts(self.addresses.main_price_route, shard);
        section.extend(second.accounts(self.addresses.second_price_route, shard));
        Ok(section)
    }

    #[must_use]
    pub fn process_equity(&self, price_section: &[AccountMeta]) -> Instruction {
        let a = &self.addresses;
        let mut accounts = vec![
            AccountMeta::new_readonly(self.args.actor, true),
            AccountMeta::new(a.vault, false),
            AccountMeta::new_readonly(a.policy, false),
            AccountMeta::new_readonly(a.delegate, false),
            AccountMeta::new(self.args.share_mint, false),
            AccountMeta::new(a.vault_main_ata, false),
            AccountMeta::new(a.vault_second_ata, false),
            AccountMeta::new(a.manager_stake_ata, false),
            AccountMeta::new_readonly(self.args.main_mint, false),
            AccountMeta::new_readonly(self.args.second_mint, false),
            AccountMeta::new_readonly(self.args.main_token_program, false),
            AccountMeta::new_readonly(self.args.second_token_program, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ];
        accounts.extend_from_slice(price_section);
        self.instruction(VaultIx::ProcessVaultEquity, accounts, &[])
    }

    #[must_use]
    pub fn process_idle_withdrawal(
        &self,
        user: Pubkey,
        price_section: &[AccountMeta],
    ) -> Instruction {
        let request = pda::withdraw_request(&self.args.program_id, &self.addresses.vault, &user).0;
        let mut ix = self.process_equity(price_section);
        ix.accounts.push(AccountMeta::new_readonly(request, false));
        ix
    }

    pub fn process_deposits<I>(&self, payouts: I) -> Result<Instruction, VaultChainError>
    where
        I: IntoIterator,
        I::Item: Into<DepositPayout>,
    {
        let payouts: Vec<DepositPayout> = payouts.into_iter().map(Into::into).collect();
        validate_batch(payouts.iter().map(|payout| &payout.user), payouts.len())?;
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let mut accounts = Vec::with_capacity(11 + payouts.len() * 7);
        accounts.extend([
            AccountMeta::new(self.args.actor, true),
            AccountMeta::new(*vault, false),
            AccountMeta::new_readonly(self.addresses.delegate, false),
            AccountMeta::new(self.args.share_mint, false),
            AccountMeta::new(self.addresses.vault_main_ata, false),
            AccountMeta::new_readonly(self.args.main_mint, false),
            AccountMeta::new_readonly(self.args.main_token_program, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(self.addresses.manager_stake_ata, false),
        ]);
        for DepositPayout {
            user,
            refund,
            shares,
        } in &payouts
        {
            let escrow = pda::deposit_escrow_authority(program_id, vault, user).0;
            let share_ata = self.share_ata(user);
            let refund = match refund {
                Some(refund) => *refund,
                None => ata(user, &self.args.main_mint, &self.args.main_token_program),
            };
            let share = match shares {
                Some(shares) => *shares,
                None => share_ata,
            };
            accounts.extend([
                AccountMeta::new(*user, false),
                AccountMeta::new(pda::deposit_request(program_id, vault, user).0, false),
                AccountMeta::new_readonly(escrow, false),
                AccountMeta::new(
                    ata(&escrow, &self.args.main_mint, &self.args.main_token_program),
                    false,
                ),
                AccountMeta::new(refund, false),
                AccountMeta::new(share, false),
                AccountMeta::new_readonly(share_ata, false),
            ]);
        }
        Ok(self.instruction(VaultIx::ProcessDeposits, accounts, &[]))
    }

    pub fn process_withdrawals<I>(&self, payouts: I) -> Result<Instruction, VaultChainError>
    where
        I: IntoIterator,
        I::Item: Into<WithdrawalPayout>,
    {
        let payouts: Vec<WithdrawalPayout> = payouts.into_iter().map(Into::into).collect();
        validate_batch(payouts.iter().map(|payout| &payout.user), payouts.len())?;
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let mut accounts = Vec::with_capacity(10 + payouts.len() * 7);
        accounts.extend([
            AccountMeta::new(self.args.actor, true),
            AccountMeta::new(*vault, false),
            AccountMeta::new_readonly(self.addresses.delegate, false),
            AccountMeta::new(self.args.share_mint, false),
            AccountMeta::new(self.addresses.vault_main_ata, false),
            AccountMeta::new_readonly(self.args.main_mint, false),
            AccountMeta::new_readonly(self.args.main_token_program, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
        ]);
        for WithdrawalPayout { user, shares_back } in &payouts {
            let escrow = pda::withdraw_escrow_authority(program_id, vault, user).0;
            let share_ata = self.share_ata(user);
            let share = match shares_back {
                Some(account) => *account,
                None => share_ata,
            };
            accounts.extend([
                AccountMeta::new(*user, false),
                AccountMeta::new(pda::withdraw_request(program_id, vault, user).0, false),
                AccountMeta::new_readonly(escrow, false),
                AccountMeta::new(
                    ata(&escrow, &self.args.share_mint, &TOKEN_PROGRAM_ID),
                    false,
                ),
                AccountMeta::new(
                    ata(user, &self.args.main_mint, &self.args.main_token_program),
                    false,
                ),
                AccountMeta::new(share, false),
                AccountMeta::new_readonly(share_ata, false),
            ]);
        }
        Ok(self.instruction(VaultIx::ProcessWithdrawals, accounts, &[]))
    }

    #[must_use]
    pub fn process_final_withdrawal(&self, payout: impl Into<WithdrawalPayout>) -> Instruction {
        let WithdrawalPayout { user, shares_back } = payout.into();
        let (program_id, a) = (&self.args.program_id, &self.addresses);
        let escrow = pda::withdraw_escrow_authority(program_id, &a.vault, &user).0;
        let share_ata = self.share_ata(&user);
        let share = match shares_back {
            Some(account) => account,
            None => share_ata,
        };
        self.instruction(
            VaultIx::ProcessFinalWithdrawal,
            vec![
                AccountMeta::new(self.args.actor, true),
                AccountMeta::new(a.vault, false),
                AccountMeta::new_readonly(a.delegate, false),
                AccountMeta::new(user, false),
                AccountMeta::new(pda::withdraw_request(program_id, &a.vault, &user).0, false),
                AccountMeta::new_readonly(escrow, false),
                AccountMeta::new(
                    ata(&escrow, &self.args.share_mint, &TOKEN_PROGRAM_ID),
                    false,
                ),
                AccountMeta::new(self.args.share_mint, false),
                AccountMeta::new(a.vault_main_ata, false),
                AccountMeta::new(a.vault_second_ata, false),
                AccountMeta::new(
                    ata(&user, &self.args.main_mint, &self.args.main_token_program),
                    false,
                ),
                AccountMeta::new(
                    ata(
                        &user,
                        &self.args.second_mint,
                        &self.args.second_token_program,
                    ),
                    false,
                ),
                AccountMeta::new_readonly(self.args.main_mint, false),
                AccountMeta::new_readonly(self.args.second_mint, false),
                AccountMeta::new_readonly(self.args.main_token_program, false),
                AccountMeta::new_readonly(self.args.second_token_program, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new(share, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(share_ata, false),
            ],
            &[],
        )
    }

    #[must_use]
    pub fn finalize_cycle(&self) -> Instruction {
        self.instruction(
            VaultIx::FinalizeVaultCycle,
            vec![
                AccountMeta::new_readonly(self.args.actor, true),
                AccountMeta::new(self.addresses.vault, false),
                AccountMeta::new_readonly(self.addresses.policy, false),
                AccountMeta::new_readonly(self.addresses.delegate, false),
                AccountMeta::new(self.addresses.vault_main_ata, false),
                AccountMeta::new_readonly(self.args.main_mint, false),
                AccountMeta::new_readonly(self.args.main_token_program, false),
            ],
            &[],
        )
    }

    #[must_use]
    pub fn deposit_batch_shape(
        &self,
        users: &[Pubkey],
        missing_share_accounts: usize,
        missing_main_refund_accounts: usize,
    ) -> CapitalBatchShape {
        self.batch_shape(
            VaultRequestKind::Deposit,
            users,
            missing_share_accounts,
            missing_main_refund_accounts,
        )
    }

    #[must_use]
    pub fn withdrawal_batch_shape(
        &self,
        users: &[Pubkey],
        missing_main_accounts: usize,
        missing_share_return_accounts: usize,
    ) -> CapitalBatchShape {
        self.batch_shape(
            VaultRequestKind::Withdraw,
            users,
            missing_share_return_accounts,
            missing_main_accounts,
        )
    }

    fn batch_shape(
        &self,
        kind: VaultRequestKind,
        users: &[Pubkey],
        missing_share: usize,
        missing_main: usize,
    ) -> CapitalBatchShape {
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let (main_mint, main_program) = (&self.args.main_mint, &self.args.main_token_program);
        let share_mint = &self.args.share_mint;
        let main_checks = if *main_program == TOKEN_2022_PROGRAM_ID {
            2
        } else {
            1
        };
        let mut common_steps = ata_steps(vault, main_mint, main_program) * main_checks;
        if kind == VaultRequestKind::Deposit {
            let (manager, manager_bump) = pda::manager_stake(program_id, vault);
            common_steps +=
                steps(manager_bump) + ata_steps(&manager, share_mint, &TOKEN_PROGRAM_ID);
        }
        let mut share_steps = [0; VAULT_CAPITAL_BATCH_USERS];
        let mut main_steps = [0; VAULT_CAPITAL_BATCH_USERS];
        let per_user: u32 = users
            .iter()
            .take(VAULT_CAPITAL_BATCH_USERS)
            .enumerate()
            .map(|(index, user)| {
                let (request_bump, escrow_bump, escrow_steps) = match kind {
                    VaultRequestKind::Deposit => {
                        let (escrow, bump) = pda::deposit_escrow_authority(program_id, vault, user);
                        (
                            pda::deposit_request(program_id, vault, user).1,
                            bump,
                            ata_steps(&escrow, main_mint, main_program) * main_checks,
                        )
                    }
                    VaultRequestKind::Withdraw => {
                        let (escrow, bump) =
                            pda::withdraw_escrow_authority(program_id, vault, user);
                        (
                            pda::withdraw_request(program_id, vault, user).1,
                            bump,
                            ata_steps(&escrow, share_mint, &TOKEN_PROGRAM_ID),
                        )
                    }
                };
                share_steps[index] = ata_steps(user, share_mint, &TOKEN_PROGRAM_ID);
                main_steps[index] = ata_steps(user, main_mint, main_program);
                steps(request_bump)
                    + steps(escrow_bump)
                    + escrow_steps
                    + main_steps[index]
                    + share_steps[index]
            })
            .sum();
        CapitalBatchShape {
            users: batch_users(users.len()),
            created_destinations: batch_users(missing_share) + batch_users(missing_main),
            derivation_steps: per_user
                + max_creation_steps(share_steps, missing_share.min(users.len()))
                + max_creation_steps(main_steps, missing_main.min(users.len()))
                + common_steps,
        }
    }

    /// Share account the program creates for `user` when the canonical share ATA cannot receive.
    #[must_use]
    pub fn share_return_account(&self, kind: VaultRequestKind, user: &Pubkey) -> Pubkey {
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let request = match kind {
            VaultRequestKind::Deposit => pda::deposit_request(program_id, vault, user).0,
            VaultRequestKind::Withdraw => pda::withdraw_request(program_id, vault, user).0,
        };
        let digest = Sha256::new()
            .chain_update(b"acta-share-return-v1")
            .chain_update(request)
            .finalize();
        let seed = hex::encode(&digest[..16]);
        Pubkey::new_from_array(
            Sha256::new()
                .chain_update(self.args.actor)
                .chain_update(seed)
                .chain_update(TOKEN_PROGRAM_ID)
                .finalize()
                .into(),
        )
    }

    fn share_ata(&self, user: &Pubkey) -> Pubkey {
        ata(user, &self.args.share_mint, &TOKEN_PROGRAM_ID)
    }

    pub(super) fn instruction(
        &self,
        ix: VaultIx,
        accounts: Vec<AccountMeta>,
        args: &[u8],
    ) -> Instruction {
        let mut data = Vec::with_capacity(1 + args.len());
        data.push(ix as u8);
        data.extend_from_slice(args);
        Instruction {
            program_id: self.args.program_id,
            accounts,
            data,
        }
    }
}

fn validate_batch<'a>(
    users: impl Iterator<Item = &'a Pubkey>,
    len: usize,
) -> Result<(), VaultChainError> {
    if len == 0 {
        return Err(VaultChainError::InvalidInput("vault batch cannot be empty"));
    }
    if len > VAULT_CAPITAL_BATCH_USERS {
        return Err(VaultChainError::BatchTooLarge);
    }
    if users.collect::<HashSet<_>>().len() != len {
        return Err(VaultChainError::InvalidInput(
            "vault batch contains duplicate users",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::max_creation_steps;

    #[test]
    fn unknown_missing_destinations_are_covered_by_the_largest_bumps() {
        let steps = [9, 1, 15];
        for mask in 0u8..8 {
            let actual = steps
                .iter()
                .enumerate()
                .filter_map(|(index, step)| (mask & (1 << index) != 0).then_some(step))
                .sum::<u32>();
            assert!(actual <= max_creation_steps(steps, mask.count_ones() as usize));
        }
    }
}
