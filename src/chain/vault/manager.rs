use std::collections::HashSet;

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::{VaultAccount, VaultChainError, pda};
use crate::chain::ix::{
    ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
    derive_associated_token_address as ata, derive_associated_token_address_with_bump,
};

/// On-chain ceiling of users per `ProcessDeposits` / `ProcessWithdrawals`.
pub const VAULT_CAPITAL_BATCH_USERS: usize = 4;

/// Users per capital transaction: a four-user batch overflows the 1232-byte packet.
pub const CAPITAL_TX_USERS: usize = 3;

const BATCH_BASE_COMPUTE_UNITS: u32 = 40_000;
const COMPUTE_UNITS_PER_USER: u32 = 13_000;
const COMPUTE_UNITS_PER_CREATED_DESTINATION: u32 = 25_000;
const COMPUTE_UNITS_PER_DERIVATION_STEP: u32 = 1_800;

// Must match on-chain instruction discriminants.
#[repr(u8)]
#[derive(Clone, Copy)]
pub(super) enum VaultIx {
    ProcessVaultEquity = 37,
    ProcessWithdrawals = 38,
    ProcessDeposits = 39,
    FinalizeVaultCycle = 40,
    SetAcceptingDeposits = 46,
    RotateVaultDelegate = 49,
    ProposeVaultUpdate = 57,
    ApplyVaultUpdate = 58,
    CancelVaultUpdate = 59,
    ProcessFinalWithdrawal = 64,
    DisableVaultDelegate = 72,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultAddresses {
    pub vault: Pubkey,
    pub policy: Pubkey,
    pub delegate: Pubkey,
    pub global_config: Pubkey,
    pub main_oracle_source: Pubkey,
    pub second_oracle_source: Pubkey,
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
        })
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
            main_oracle_source: pda::oracle_source(program_id, &args.main_mint).0,
            second_oracle_source: pda::oracle_source(program_id, &args.second_mint).0,
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

    #[must_use]
    pub fn process_equity(
        &self,
        main_price_update: Pubkey,
        second_price_update: Pubkey,
    ) -> Instruction {
        let a = &self.addresses;
        self.instruction(
            VaultIx::ProcessVaultEquity,
            vec![
                AccountMeta::new_readonly(self.args.actor, true),
                AccountMeta::new(a.vault, false),
                AccountMeta::new_readonly(a.policy, false),
                AccountMeta::new_readonly(a.delegate, false),
                AccountMeta::new(self.args.share_mint, false),
                AccountMeta::new(a.vault_main_ata, false),
                AccountMeta::new(a.vault_second_ata, false),
                AccountMeta::new_readonly(a.manager_stake_authority, false),
                AccountMeta::new(a.manager_stake_ata, false),
                AccountMeta::new_readonly(self.args.main_mint, false),
                AccountMeta::new_readonly(a.main_oracle_source, false),
                AccountMeta::new_readonly(main_price_update, false),
                AccountMeta::new_readonly(self.args.second_mint, false),
                AccountMeta::new_readonly(a.second_oracle_source, false),
                AccountMeta::new_readonly(second_price_update, false),
                AccountMeta::new_readonly(self.args.main_token_program, false),
                AccountMeta::new_readonly(self.args.second_token_program, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            &[],
        )
    }

    #[must_use]
    pub fn process_idle_withdrawal(
        &self,
        user: Pubkey,
        main_price_update: Pubkey,
        second_price_update: Pubkey,
    ) -> Instruction {
        let request = pda::withdraw_request(&self.args.program_id, &self.addresses.vault, &user).0;
        let mut ix = self.process_equity(main_price_update, second_price_update);
        ix.accounts.push(AccountMeta::new_readonly(request, false));
        ix
    }

    pub fn process_deposits(&self, users: &[Pubkey]) -> Result<Instruction, VaultChainError> {
        let users: Vec<_> = users.iter().map(|user| (*user, None, None)).collect();
        self.process_deposits_with_destinations(&users)
    }

    /// `ProcessDeposits` with `(user, main refund override, share recipient override)`.
    pub fn process_deposits_with_destinations(
        &self,
        users: &[(Pubkey, Option<Pubkey>, Option<Pubkey>)],
    ) -> Result<Instruction, VaultChainError> {
        validate_batch(users.iter().map(|(user, _, _)| user), users.len())?;
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let mut accounts = Vec::with_capacity(11 + users.len() * 7);
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
        for (user, refund, share) in users {
            let escrow = pda::deposit_escrow_authority(program_id, vault, user).0;
            let share_ata = self.share_ata(user);
            let refund = match refund {
                Some(refund) => *refund,
                None => ata(user, &self.args.main_mint, &self.args.main_token_program),
            };
            let share = match share {
                Some(share) => *share,
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

    pub fn process_deposit_batches(
        &self,
        users: &[Pubkey],
    ) -> Result<Vec<Instruction>, VaultChainError> {
        validate_users(users.iter(), users.len())?;
        users
            .chunks(CAPITAL_TX_USERS)
            .map(|batch| self.process_deposits(batch))
            .collect()
    }

    pub fn process_withdrawals(&self, users: &[Pubkey]) -> Result<Instruction, VaultChainError> {
        let users: Vec<_> = users.iter().map(|user| (*user, None)).collect();
        self.process_withdrawals_with_share_destinations(&users)
    }

    /// `ProcessWithdrawals` with `(user, share return override)`.
    pub fn process_withdrawals_with_share_destinations(
        &self,
        users: &[(Pubkey, Option<Pubkey>)],
    ) -> Result<Instruction, VaultChainError> {
        validate_batch(users.iter().map(|(user, _)| user), users.len())?;
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let mut accounts = Vec::with_capacity(10 + users.len() * 7);
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
        for (user, share) in users {
            let escrow = pda::withdraw_escrow_authority(program_id, vault, user).0;
            let share_ata = self.share_ata(user);
            let share = match share {
                Some(share) => *share,
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

    pub fn process_withdrawal_batches(
        &self,
        users: &[Pubkey],
    ) -> Result<Vec<Instruction>, VaultChainError> {
        validate_users(users.iter(), users.len())?;
        users
            .chunks(CAPITAL_TX_USERS)
            .map(|batch| self.process_withdrawals(batch))
            .collect()
    }

    #[must_use]
    pub fn process_final_withdrawal(&self, user: Pubkey) -> Instruction {
        self.process_final_withdrawal_with_share_destination(user, None)
    }

    #[must_use]
    pub fn process_final_withdrawal_with_share_destination(
        &self,
        user: Pubkey,
        share_destination: Option<Pubkey>,
    ) -> Instruction {
        let (program_id, a) = (&self.args.program_id, &self.addresses);
        let escrow = pda::withdraw_escrow_authority(program_id, &a.vault, &user).0;
        let share_ata = self.share_ata(&user);
        let share = match share_destination {
            Some(share) => share,
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
    pub fn deposit_batch_shape_conservative(&self, users: &[Pubkey]) -> CapitalBatchShape {
        self.deposit_batch_shape_with_destinations(users, users.len(), users.len())
    }

    pub(super) fn deposit_batch_shape_with_destinations(
        &self,
        users: &[Pubkey],
        missing_share_accounts: usize,
        missing_main_refund_accounts: usize,
    ) -> CapitalBatchShape {
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let (main_mint, main_program) = (&self.args.main_mint, &self.args.main_token_program);
        let main_checks = if *main_program == TOKEN_2022_PROGRAM_ID {
            2
        } else {
            1
        };
        let (manager, manager_bump) = pda::manager_stake(program_id, vault);
        let common_steps = steps(manager_bump)
            + ata_steps(&manager, &self.args.share_mint, &TOKEN_PROGRAM_ID)
            + ata_steps(vault, main_mint, main_program) * main_checks;
        let mut share_steps = [0; VAULT_CAPITAL_BATCH_USERS];
        let mut main_steps = [0; VAULT_CAPITAL_BATCH_USERS];
        let per_user: u32 = users
            .iter()
            .take(VAULT_CAPITAL_BATCH_USERS)
            .enumerate()
            .map(|(index, user)| {
                let request_bump = pda::deposit_request(program_id, vault, user).1;
                let (escrow, escrow_bump) = pda::deposit_escrow_authority(program_id, vault, user);
                share_steps[index] = ata_steps(user, &self.args.share_mint, &TOKEN_PROGRAM_ID);
                main_steps[index] = ata_steps(user, main_mint, main_program);
                steps(request_bump)
                    + steps(escrow_bump)
                    + ata_steps(&escrow, main_mint, main_program) * main_checks
                    + main_steps[index]
                    + share_steps[index]
            })
            .sum();
        CapitalBatchShape {
            users: batch_users(users.len()),
            created_destinations: batch_users(missing_share_accounts)
                + batch_users(missing_main_refund_accounts),
            derivation_steps: per_user
                + max_creation_steps(share_steps, missing_share_accounts.min(users.len()))
                + max_creation_steps(main_steps, missing_main_refund_accounts.min(users.len()))
                + common_steps,
        }
    }

    #[must_use]
    pub fn withdrawal_batch_shape_conservative(&self, users: &[Pubkey]) -> CapitalBatchShape {
        self.withdrawal_batch_shape_with_destinations(users, users.len(), users.len())
    }

    pub(super) fn withdrawal_batch_shape_with_destinations(
        &self,
        users: &[Pubkey],
        missing_main_accounts: usize,
        missing_share_return_accounts: usize,
    ) -> CapitalBatchShape {
        let (program_id, vault) = (&self.args.program_id, &self.addresses.vault);
        let (main_mint, main_program) = (&self.args.main_mint, &self.args.main_token_program);
        let share_mint = &self.args.share_mint;
        let main_checks = if *main_program == TOKEN_2022_PROGRAM_ID {
            2
        } else {
            1
        };
        let common_steps = ata_steps(vault, main_mint, main_program) * main_checks;
        let mut main_steps = [0; VAULT_CAPITAL_BATCH_USERS];
        let mut share_steps = [0; VAULT_CAPITAL_BATCH_USERS];
        let per_user: u32 = users
            .iter()
            .take(VAULT_CAPITAL_BATCH_USERS)
            .enumerate()
            .map(|(index, user)| {
                let request_bump = pda::withdraw_request(program_id, vault, user).1;
                let (escrow, escrow_bump) = pda::withdraw_escrow_authority(program_id, vault, user);
                main_steps[index] = ata_steps(user, main_mint, main_program);
                share_steps[index] = ata_steps(user, share_mint, &TOKEN_PROGRAM_ID);
                steps(request_bump)
                    + steps(escrow_bump)
                    + ata_steps(&escrow, share_mint, &TOKEN_PROGRAM_ID)
                    + main_steps[index]
                    + share_steps[index]
            })
            .sum();
        CapitalBatchShape {
            users: batch_users(users.len()),
            created_destinations: batch_users(missing_main_accounts)
                + batch_users(missing_share_return_accounts),
            derivation_steps: per_user
                + max_creation_steps(main_steps, missing_main_accounts.min(users.len()))
                + max_creation_steps(share_steps, missing_share_return_accounts.min(users.len()))
                + common_steps,
        }
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
    validate_users(users, len)?;
    if len > VAULT_CAPITAL_BATCH_USERS {
        return Err(VaultChainError::BatchTooLarge);
    }
    Ok(())
}

fn validate_users<'a>(
    users: impl Iterator<Item = &'a Pubkey>,
    len: usize,
) -> Result<(), VaultChainError> {
    if len == 0 {
        return Err(VaultChainError::InvalidInput("vault batch cannot be empty"));
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
        let steps = [1, 9, 3, 15];
        for mask in 0u8..16 {
            let actual = steps
                .iter()
                .enumerate()
                .filter_map(|(index, step)| (mask & (1 << index) != 0).then_some(step))
                .sum::<u32>();
            assert!(actual <= max_creation_steps(steps, mask.count_ones() as usize));
        }
    }
}
