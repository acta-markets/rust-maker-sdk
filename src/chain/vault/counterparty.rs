use solana_sdk::pubkey::Pubkey;

use super::{VaultAccount, VaultManager, VaultOpenIntent, VaultSaleTerms};
use crate::chain::{ChainClient, ChainError, MakerInfo, maker_pda_with_program_id};

const VAULT_DISCRIMINATOR: u8 = 9;

/// The other side of a vault sale: an external maker or another vault.
#[derive(Debug, Clone)]
pub enum Counterparty {
    Maker { owner: Pubkey, quote_signer: Pubkey },
    Vault(Box<VaultManager>),
}

impl Counterparty {
    /// Resolves a quote's `maker` from chain: a vault PDA owned by the program, else the owner's maker account.
    pub async fn load(chain: &ChainClient, owner: Pubkey) -> Result<Self, ChainError> {
        let program = chain.program_id();
        if let Some(account) = chain.fetch_optional_account(&owner).await?
            && account.owner == program
            && account.data.first() == Some(&VAULT_DISCRIMINATOR)
        {
            let vault = VaultAccount::from_account(&account, &program)?;
            let holder = VaultManager::from_vault(program, vault.primary_delegate, &vault)?;
            if holder.addresses.vault != owner {
                return Err(ChainError::Identity("holder vault PDA"));
            }
            return Ok(Self::Vault(Box::new(holder)));
        }
        let maker_pda = maker_pda_with_program_id(&program, &owner).0;
        let maker = MakerInfo::from_account(&chain.fetch_account(&maker_pda).await?, &program)?;
        if maker.owner != owner {
            return Err(ChainError::Identity("maker owner"));
        }
        Ok(Self::Maker {
            owner,
            quote_signer: maker.quote_signer,
        })
    }

    /// The open the writer signs when selling to this counterparty.
    #[must_use]
    pub fn sale<'a>(
        &'a self,
        writer: &'a VaultManager,
        keeper: Pubkey,
        terms: VaultSaleTerms,
    ) -> VaultOpenIntent<'a> {
        match self {
            Self::Maker {
                owner,
                quote_signer,
            } => VaultOpenIntent::Sale {
                keeper,
                writer,
                maker_owner: *owner,
                maker_quote_signer: *quote_signer,
                terms,
            },
            Self::Vault(holder) => VaultOpenIntent::BetweenVaults {
                keeper,
                writer,
                holder,
                terms,
            },
        }
    }
}
