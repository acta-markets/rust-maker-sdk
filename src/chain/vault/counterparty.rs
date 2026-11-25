use solana_pubkey::Pubkey;

use super::{VaultAccount, VaultManager, VaultOpenIntent, VaultOpenTerms};
use crate::chain::{ChainClient, ChainError, MakerInfo, maker_pda_with_program_id};
use crate::types::TakerSide;

const VAULT_DISCRIMINATOR: u8 = 9;

/// The other side of a vault trade: an external maker or another vault.
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

    /// The open `vault` signs when it trades `side` as taker with this counterparty.
    #[must_use]
    pub fn open<'a>(
        &'a self,
        vault: &'a VaultManager,
        keeper: Pubkey,
        side: TakerSide,
        terms: VaultOpenTerms,
    ) -> VaultOpenIntent<'a> {
        match side {
            TakerSide::Sell => self.sale(vault, keeper, terms),
            TakerSide::Buy => self.purchase(vault, keeper, terms),
        }
    }

    /// The open the writer signs when selling to this counterparty.
    #[must_use]
    pub fn sale<'a>(
        &'a self,
        writer: &'a VaultManager,
        keeper: Pubkey,
        terms: VaultOpenTerms,
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

    /// The open the holder signs when buying what this counterparty writes.
    #[must_use]
    pub fn purchase<'a>(
        &'a self,
        holder: &'a VaultManager,
        keeper: Pubkey,
        terms: VaultOpenTerms,
    ) -> VaultOpenIntent<'a> {
        match self {
            Self::Maker {
                owner,
                quote_signer,
            } => VaultOpenIntent::Purchase {
                keeper,
                holder,
                maker_owner: *owner,
                maker_quote_signer: *quote_signer,
                terms,
            },
            Self::Vault(writer) => VaultOpenIntent::PurchaseFromVault {
                keeper,
                writer,
                holder,
                terms,
            },
        }
    }
}
