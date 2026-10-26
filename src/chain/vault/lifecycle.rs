use super::{VaultChainError, VaultManager, manager::VaultIx};
use crate::chain::ix::{TOKEN_PROGRAM_ID, derive_associated_token_address as ata};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

impl VaultManager {
    pub fn bootstrap(
        &self,
        fee_recipient: Pubkey,
        amount: u64,
    ) -> Result<Instruction, VaultChainError> {
        if amount < 100_000_000 || fee_recipient == Pubkey::default() {
            return Err(VaultChainError::InvalidInput(
                "bootstrap requires 100000000 main atoms and a nonzero recipient",
            ));
        }
        let a = &self.addresses;
        let p = &self.args;
        let mut data = amount.to_le_bytes().to_vec();
        data.extend_from_slice(fee_recipient.as_ref());
        Ok(self.instruction(
            VaultIx::Bootstrap,
            vec![
                AccountMeta::new_readonly(p.actor, true),
                AccountMeta::new(a.vault, false),
                AccountMeta::new(p.share_mint, false),
                AccountMeta::new(a.manager_stake_ata, false),
                AccountMeta::new(ata(&p.actor, &p.main_mint, &p.main_token_program), false),
                AccountMeta::new(a.vault_main_ata, false),
                AccountMeta::new_readonly(p.main_mint, false),
                AccountMeta::new_readonly(p.main_token_program, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            &data,
        ))
    }

    pub fn top_up_manager_stake(
        &self,
        amount: u64,
        prices: &[AccountMeta],
    ) -> Result<Instruction, VaultChainError> {
        if amount == 0 {
            return Err(VaultChainError::InvalidInput(
                "top-up amount must be positive",
            ));
        }
        if prices.is_empty() {
            return Err(VaultChainError::MissingPriceSection);
        }
        let mut accounts = self.owner_equity_accounts();
        accounts.push(AccountMeta::new(
            ata(
                &self.args.actor,
                &self.args.main_mint,
                &self.args.main_token_program,
            ),
            false,
        ));
        accounts.extend_from_slice(prices);
        Ok(self.instruction(VaultIx::TopUpManagerStake, accounts, &amount.to_le_bytes()))
    }

    pub fn unlock_manager_stake(
        &self,
        fee_recipient: Pubkey,
        shares: u64,
    ) -> Result<Instruction, VaultChainError> {
        let destination = ata(&fee_recipient, &self.args.share_mint, &TOKEN_PROGRAM_ID);
        if shares == 0 || destination == self.addresses.manager_stake_ata {
            return Err(VaultChainError::InvalidInput(
                "unlock requires positive shares and a distinct destination",
            ));
        }
        let a = &self.addresses;
        Ok(self.instruction(
            VaultIx::UnlockManagerStake,
            vec![
                AccountMeta::new_readonly(self.args.actor, true),
                AccountMeta::new(a.vault, false),
                AccountMeta::new_readonly(a.manager_stake_authority, false),
                AccountMeta::new(a.manager_stake_ata, false),
                AccountMeta::new(destination, false),
                AccountMeta::new_readonly(self.args.share_mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            &shares.to_le_bytes(),
        ))
    }

    pub fn unlock_manager_stake_sequence(
        &self,
        fee_recipient: Pubkey,
        shares: u64,
    ) -> Result<Vec<Instruction>, VaultChainError> {
        let unlock = self.unlock_manager_stake(fee_recipient, shares)?;
        Ok(vec![
            create_destination_ata(
                self.args.actor,
                fee_recipient,
                self.args.share_mint,
                TOKEN_PROGRAM_ID,
            ),
            unlock,
        ])
    }

    #[must_use]
    pub fn accept_governance(&self) -> Instruction {
        self.owner_instruction(VaultIx::AcceptGovernance, &[])
    }

    pub fn revoke_governance_nomination(
        &self,
        expected_nominee: Pubkey,
    ) -> Result<Instruction, VaultChainError> {
        if expected_nominee == Pubkey::default() {
            return Err(VaultChainError::InvalidInput(
                "expected nominee must be nonzero",
            ));
        }
        Ok(self.owner_instruction(
            VaultIx::RevokeGovernanceNomination,
            expected_nominee.as_ref(),
        ))
    }

    pub fn apply_fee_recipient_update(
        &self,
        expected: Pubkey,
        recipient: Pubkey,
    ) -> Result<Instruction, VaultChainError> {
        if expected == recipient || expected == Pubkey::default() || recipient == Pubkey::default()
        {
            return Err(VaultChainError::InvalidInput(
                "recipient must change between nonzero keys",
            ));
        }
        let mut data = expected.to_bytes().to_vec();
        data.extend_from_slice(recipient.as_ref());
        Ok(self.owner_instruction(VaultIx::ApplyFeeRecipientUpdate, &data))
    }

    #[must_use]
    pub fn start_shutdown(&self) -> Instruction {
        self.owner_instruction(VaultIx::StartShutdown, &[])
    }

    /// Pass no prices when second custody is empty; otherwise supply both routes.
    #[must_use]
    pub fn finalize_shutdown(&self, prices: &[AccountMeta]) -> Instruction {
        let mut accounts = self.owner_equity_accounts();
        accounts.extend_from_slice(prices);
        self.instruction(VaultIx::FinalizeShutdown, accounts, &[])
    }

    #[must_use]
    pub fn close_vault(&self) -> Instruction {
        let a = &self.addresses;
        let p = &self.args;
        self.instruction(
            VaultIx::CloseVault,
            vec![
                AccountMeta::new(p.actor, true),
                AccountMeta::new(a.vault, false),
                AccountMeta::new(a.delegate, false),
                AccountMeta::new(a.policy, false),
                AccountMeta::new_readonly(p.share_mint, false),
                AccountMeta::new(a.vault_main_ata, false),
                AccountMeta::new(a.vault_second_ata, false),
                AccountMeta::new_readonly(p.main_mint, false),
                AccountMeta::new_readonly(p.second_mint, false),
                AccountMeta::new_readonly(p.main_token_program, false),
                AccountMeta::new_readonly(p.second_token_program, false),
                AccountMeta::new_readonly(a.manager_stake_authority, false),
                AccountMeta::new(a.manager_stake_ata, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new(ata(&p.actor, &p.main_mint, &p.main_token_program), false),
                AccountMeta::new(
                    ata(&p.actor, &p.second_mint, &p.second_token_program),
                    false,
                ),
            ],
            &[],
        )
    }

    /// Creates the governance destination ATAs before closing custody.
    #[must_use]
    pub fn close_vault_sequence(&self) -> Vec<Instruction> {
        let p = &self.args;
        let mut instructions = vec![
            create_destination_ata(p.actor, p.actor, p.main_mint, p.main_token_program),
            create_destination_ata(p.actor, p.actor, p.second_mint, p.second_token_program),
        ];
        instructions.push(self.close_vault());
        instructions
    }

    fn owner_instruction(&self, kind: VaultIx, data: &[u8]) -> Instruction {
        self.instruction(
            kind,
            vec![
                AccountMeta::new_readonly(self.args.actor, true),
                AccountMeta::new(self.addresses.vault, false),
            ],
            data,
        )
    }

    fn owner_equity_accounts(&self) -> Vec<AccountMeta> {
        let mut accounts = self.process_equity(&[]).accounts;
        accounts.remove(3);
        accounts
    }
}

pub(super) fn create_destination_ata(
    payer: Pubkey,
    owner: Pubkey,
    mint: Pubkey,
    program: Pubkey,
) -> Instruction {
    use crate::chain::ix::{ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID};
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(ata(&owner, &mint, &program), false),
            AccountMeta::new_readonly(owner, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(program, false),
        ],
        data: vec![1],
    }
}
