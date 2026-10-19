use solana_rpc_client::api::filter::{Memcmp, RpcFilterType};
use solana_sdk::pubkey::Pubkey;

use super::{VaultRequest, VaultRequestKind};
use crate::chain::{ChainClient, ChainError, PositionInfo};

const POSITION_DISCRIMINATOR: u8 = 1;
const POSITION_LEN: u64 = 424;
const POSITION_OFFSET_STATUS: usize = 4;
const POSITION_OFFSET_MAKER_OWNER: usize = 40;
const POSITION_OPEN: u8 = 1;

/// Queued capital requests: (user, main-mint atoms) deposits and (user, shares) withdrawals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingCapitalRequests {
    pub deposits: Vec<(Pubkey, u64)>,
    pub withdrawals: Vec<(Pubkey, u64)>,
}

impl ChainClient {
    /// Positions the vault bought as a maker that still await
    /// `VaultFundMakerPosition`, read from the program's accounts.
    pub async fn unfunded_positions(
        &self,
        vault: &Pubkey,
    ) -> Result<Vec<(Pubkey, PositionInfo)>, ChainError> {
        let accounts = self
            .program_accounts(vec![
                RpcFilterType::DataSize(POSITION_LEN),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(0, &[POSITION_DISCRIMINATOR])),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                    POSITION_OFFSET_STATUS,
                    &[POSITION_OPEN],
                )),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                    POSITION_OFFSET_MAKER_OWNER,
                    vault.as_ref(),
                )),
            ])
            .await?;
        accounts
            .into_iter()
            .map(|(address, account)| {
                Ok((
                    address,
                    PositionInfo::from_account(&account, &self.program_id())?,
                ))
            })
            .collect()
    }

    /// The vault's nonzero queued deposits and withdrawals, read from the program's accounts.
    pub async fn pending_capital_requests(
        &self,
        vault: &Pubkey,
    ) -> Result<PendingCapitalRequests, ChainError> {
        Ok(PendingCapitalRequests {
            deposits: self.requests(vault, VaultRequestKind::Deposit).await?,
            withdrawals: self.requests(vault, VaultRequestKind::Withdraw).await?,
        })
    }

    async fn requests(
        &self,
        vault: &Pubkey,
        kind: VaultRequestKind,
    ) -> Result<Vec<(Pubkey, u64)>, ChainError> {
        let accounts = self
            .program_accounts(vec![
                RpcFilterType::DataSize(VaultRequest::LEN as u64),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(0, &[kind.discriminator()])),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                    VaultRequest::VAULT_OFFSET,
                    vault.as_ref(),
                )),
            ])
            .await?;
        let mut requests = Vec::with_capacity(accounts.len());
        for (_, account) in accounts {
            let request = VaultRequest::from_account(&account, &self.program_id(), kind)?;
            if request.vault != *vault {
                return Err(ChainError::Identity("request vault"));
            }
            if request.amount > 0 {
                requests.push((request.user, request.amount));
            }
        }
        Ok(requests)
    }
}

#[cfg(test)]
mod tests {
    use solana_sdk::account::Account;

    use super::*;
    use crate::chain::PositionStatus;

    fn account(owner: Pubkey, data: Vec<u8>) -> Account {
        Account {
            lamports: 1,
            data,
            owner,
            executable: false,
            rent_epoch: 0,
        }
    }

    fn position(program: Pubkey, maker: Pubkey, status: u8, version: u8) -> Account {
        let mut data = vec![0; POSITION_LEN as usize];
        data[..2].copy_from_slice(&[POSITION_DISCRIMINATOR, version]);
        data[POSITION_OFFSET_STATUS] = status;
        data[POSITION_OFFSET_MAKER_OWNER..POSITION_OFFSET_MAKER_OWNER + 32]
            .copy_from_slice(maker.as_ref());
        account(program, data)
    }

    fn request(
        program: Pubkey,
        kind: VaultRequestKind,
        vault: Pubkey,
        user: Pubkey,
        amount: u64,
    ) -> Account {
        let mut data = vec![0; VaultRequest::LEN];
        data[..2].copy_from_slice(&[kind.discriminator(), 1]);
        data[8..40].copy_from_slice(user.as_ref());
        data[40..72].copy_from_slice(vault.as_ref());
        data[72..80].copy_from_slice(&amount.to_le_bytes());
        account(program, data)
    }

    #[tokio::test]
    async fn unfunded_positions_are_the_vaults_open_maker_positions_on_chain() {
        let (program, vault) = (Pubkey::new_unique(), Pubkey::new_unique());
        let open = Pubkey::new_unique();
        let chain = ChainClient::with_accounts(
            program,
            [
                (open, position(program, vault, POSITION_OPEN, 2)),
                (Pubkey::new_unique(), position(program, vault, 2, 2)),
                (
                    Pubkey::new_unique(),
                    position(program, Pubkey::new_unique(), POSITION_OPEN, 2),
                ),
                (
                    Pubkey::new_unique(),
                    position(Pubkey::new_unique(), vault, POSITION_OPEN, 2),
                ),
            ],
        );
        let unfunded = chain.unfunded_positions(&vault).await.unwrap();
        let [(address, info)] = unfunded.as_slice() else {
            panic!("one open maker position: {unfunded:?}");
        };
        assert_eq!(*address, open);
        assert_eq!(info.status, PositionStatus::Open);
        assert_eq!(info.maker_owner, vault);

        let stale_layout = ChainClient::with_accounts(
            program,
            [(
                Pubkey::new_unique(),
                position(program, vault, POSITION_OPEN, 1),
            )],
        );
        assert!(matches!(
            stale_layout.unfunded_positions(&vault).await,
            Err(ChainError::InvalidAccountData)
        ));
    }

    #[tokio::test]
    async fn pending_capital_requests_are_the_vaults_nonzero_requests_on_chain() {
        let (program, vault) = (Pubkey::new_unique(), Pubkey::new_unique());
        let (first, second) = (Pubkey::new_unique(), Pubkey::new_unique());
        let chain = ChainClient::with_accounts(
            program,
            [
                (
                    Pubkey::new_unique(),
                    request(program, VaultRequestKind::Deposit, vault, first, 500),
                ),
                (
                    Pubkey::new_unique(),
                    request(program, VaultRequestKind::Deposit, vault, second, 0),
                ),
                (
                    Pubkey::new_unique(),
                    request(
                        program,
                        VaultRequestKind::Deposit,
                        Pubkey::new_unique(),
                        second,
                        9,
                    ),
                ),
                (
                    Pubkey::new_unique(),
                    request(program, VaultRequestKind::Withdraw, vault, second, 40),
                ),
            ],
        );
        assert_eq!(
            chain.pending_capital_requests(&vault).await.unwrap(),
            PendingCapitalRequests {
                deposits: vec![(first, 500)],
                withdrawals: vec![(second, 40)],
            }
        );

        let mut malformed = request(program, VaultRequestKind::Withdraw, vault, first, 1);
        malformed.data[1] = 9;
        let chain = ChainClient::with_accounts(program, [(Pubkey::new_unique(), malformed)]);
        assert!(matches!(
            chain.pending_capital_requests(&vault).await,
            Err(ChainError::Vault(_))
        ));
    }
}
