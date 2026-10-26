use super::{PriceRoute, VaultAccount, VaultChainError, VaultGovernanceUpdateKind, VaultManager};
use crate::chain::{ChainClient, ChainError};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

impl ChainClient {
    pub async fn vault_manager(
        &self,
        vault: Pubkey,
        actor: Pubkey,
    ) -> Result<VaultManager, ChainError> {
        let state =
            VaultAccount::from_account(&self.fetch_account(&vault).await?, &self.program_id())?;
        let manager = VaultManager::from_vault(self.program_id(), actor, &state)?;
        if manager.addresses.vault != vault {
            return Err(ChainError::Identity("vault PDA"));
        }
        Ok(manager)
    }
}

impl VaultManager {
    async fn current_owner_state(&self, chain: &ChainClient) -> Result<VaultAccount, ChainError> {
        if chain.program_id() != self.args.program_id {
            return Err(ChainError::Identity("vault program"));
        }
        let state = VaultAccount::from_account(
            &chain.fetch_account(&self.addresses.vault).await?,
            &chain.program_id(),
        )?;
        self.ensure_current(&state)?;
        if state.primary_delegate != self.args.delegate_authority {
            return Err(ChainError::Identity("vault delegate"));
        }
        Ok(state)
    }

    pub async fn fetch_price_section(
        &self,
        chain: &ChainClient,
        shard: u16,
    ) -> Result<Vec<AccountMeta>, ChainError> {
        self.current_owner_state(chain).await?;
        self.load_price_section(chain, shard).await
    }

    async fn load_price_section(
        &self,
        chain: &ChainClient,
        shard: u16,
    ) -> Result<Vec<AccountMeta>, ChainError> {
        let main = PriceRoute::from_account(
            &chain
                .fetch_account(&self.addresses.main_price_route)
                .await?,
            &chain.program_id(),
        )?;
        let second = PriceRoute::from_account(
            &chain
                .fetch_account(&self.addresses.second_price_route)
                .await?,
            &chain.program_id(),
        )?;
        Ok(self.price_section(&main, &second, shard)?)
    }

    pub async fn top_up_manager_stake_from_chain(
        &self,
        chain: &ChainClient,
        amount: u64,
        shard: u16,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        self.require_governance(&state)?;
        require_unstaged(&state)?;
        Ok(self.top_up_manager_stake(amount, &self.load_price_section(chain, shard).await?)?)
    }

    pub async fn finalize_shutdown_from_chain(
        &self,
        chain: &ChainClient,
        shard: u16,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        require_unstaged(&state)?;
        let prices = self.prices_for_second_balance(chain, shard).await?;
        Ok(self.finalize_shutdown(&prices))
    }

    pub async fn unlock_manager_stake_from_chain(
        &self,
        chain: &ChainClient,
        shares: u64,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        self.require_governance(&state)?;
        Ok(self.unlock_manager_stake(state.fee_recipient, shares)?)
    }

    pub async fn apply_fee_recipient_update_from_chain(
        &self,
        chain: &ChainClient,
        recipient: Pubkey,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        self.require_governance(&state)?;
        Ok(self.apply_fee_recipient_update(state.fee_recipient, recipient)?)
    }

    pub async fn revoke_governance_nomination_from_chain(
        &self,
        chain: &ChainClient,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        self.require_governance(&state)?;
        Ok(self.revoke_governance_nomination(state.pending_governance_authority)?)
    }

    /// Resolves the stored proposal payer and payload; priced kinds fetch the current routes.
    pub async fn apply_update_from_chain(
        &self,
        chain: &ChainClient,
        kind: VaultGovernanceUpdateKind,
        shard: u16,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        if matches!(
            kind,
            VaultGovernanceUpdateKind::Fees
                | VaultGovernanceUpdateKind::HwmReset
                | VaultGovernanceUpdateKind::SecondLeg
        ) {
            require_unstaged(&state)?;
        }
        let (payer, payload) = self.fetch_proposal(chain, kind).await?;
        if kind == VaultGovernanceUpdateKind::SecondLeg {
            let prices = self.prices_for_second_balance(chain, shard).await?;
            return Ok(self.apply_second_leg(
                payer,
                Pubkey::new_from_array(payload),
                Some(&prices),
            )?);
        }
        let prices = if matches!(
            kind,
            VaultGovernanceUpdateKind::Fees | VaultGovernanceUpdateKind::HwmReset
        ) {
            Some(self.load_price_section(chain, shard).await?)
        } else {
            None
        };
        Ok(self.apply_update(kind, payer, prices.as_deref())?)
    }

    pub async fn cancel_update_from_chain(
        &self,
        chain: &ChainClient,
        kind: VaultGovernanceUpdateKind,
    ) -> Result<Instruction, ChainError> {
        if kind == VaultGovernanceUpdateKind::HwmReset {
            return Err(VaultChainError::ColdAuthorityRequired.into());
        }
        self.current_owner_state(chain).await?;
        let (payer, _) = self.fetch_proposal(chain, kind).await?;
        Ok(self.cancel_update(kind, payer))
    }

    pub async fn unlock_manager_stake_sequence_from_chain(
        &self,
        chain: &ChainClient,
        shares: u64,
    ) -> Result<Vec<Instruction>, ChainError> {
        let state = self.current_owner_state(chain).await?;
        self.require_governance(&state)?;
        Ok(self.unlock_manager_stake_sequence(state.fee_recipient, shares)?)
    }

    pub async fn start_shutdown_from_chain(
        &self,
        chain: &ChainClient,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        self.require_governance(&state)?;
        require_unstaged(&state)?;
        Ok(self.start_shutdown())
    }

    pub async fn close_vault_from_chain(
        &self,
        chain: &ChainClient,
    ) -> Result<Instruction, ChainError> {
        let state = self.current_owner_state(chain).await?;
        self.require_governance(&state)?;
        require_unstaged(&state)?;
        Ok(self.close_vault())
    }

    fn require_governance(&self, state: &VaultAccount) -> Result<(), VaultChainError> {
        if self.args.actor != state.governance_authority {
            return Err(VaultChainError::GovernanceAuthorityRequired);
        }
        Ok(())
    }

    async fn prices_for_second_balance(
        &self,
        chain: &ChainClient,
        shard: u16,
    ) -> Result<Vec<AccountMeta>, ChainError> {
        let account = chain
            .fetch_account(&self.addresses.vault_second_ata)
            .await?;
        let data = &account.data;
        if account.owner != self.args.second_token_program
            || data.len() < 165
            || data[..32] != self.args.second_mint.to_bytes()
            || data[32..64] != self.addresses.vault.to_bytes()
            || data[108] != 1
        {
            return Err(ChainError::InvalidAccountData);
        }
        let amount = u64::from_le_bytes(
            data[64..72]
                .try_into()
                .map_err(|_| ChainError::InvalidAccountData)?,
        );
        let amount = match &data[109..113] {
            [0, 0, 0, 0] => amount,
            [1, 0, 0, 0] => {
                let reserve = u64::from_le_bytes(
                    data[113..121]
                        .try_into()
                        .map_err(|_| ChainError::InvalidAccountData)?,
                );
                account
                    .lamports
                    .checked_sub(reserve)
                    .filter(|synced| *synced >= amount)
                    .ok_or(ChainError::InvalidAccountData)?
            }
            _ => return Err(ChainError::InvalidAccountData),
        };
        if amount == 0 {
            Ok(Vec::new())
        } else {
            self.load_price_section(chain, shard).await
        }
    }

    async fn fetch_proposal(
        &self,
        chain: &ChainClient,
        kind: VaultGovernanceUpdateKind,
    ) -> Result<(Pubkey, [u8; 32]), ChainError> {
        let (address, bump) = super::pda::pending_vault_update(
            &self.args.program_id,
            &self.addresses.vault,
            kind as u8,
        );
        let account = chain.fetch_account(&address).await?;
        if account.owner != chain.program_id() {
            return Err(ChainError::AccountOwnerMismatch {
                expected: chain.program_id(),
                actual: account.owner,
            });
        }
        let data = &account.data;
        if data.len() != 368
            || data[0] != 14
            || data[1] != 1
            || data[2] != bump
            || data[3] != kind as u8
            || data[8..40] != self.addresses.vault.to_bytes()
            || data[40..72] == [0; 32]
        {
            return Err(ChainError::InvalidAccountData);
        }
        let payer = data[40..72]
            .try_into()
            .map_err(|_| ChainError::InvalidAccountData)?;
        let payload = data[80..112]
            .try_into()
            .map_err(|_| ChainError::InvalidAccountData)?;
        Ok((Pubkey::new_from_array(payer), payload))
    }
}

fn require_unstaged(state: &VaultAccount) -> Result<(), VaultChainError> {
    if state.staging.open.is_some() {
        return Err(VaultChainError::Staged);
    }
    Ok(())
}
