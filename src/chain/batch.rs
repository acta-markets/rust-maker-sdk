#[cfg(feature = "chain-rpc")]
use std::collections::BTreeMap;

use solana_sdk::{account::Account, instruction::Instruction, pubkey::Pubkey};

use super::{ChainError, MarketInfo, PositionInfo, PositionStatus, ix};

#[derive(Debug, Clone)]
pub struct PositionSnapshot {
    pub address: Pubkey,
    pub position: PositionInfo,
    pub market_address: Pubkey,
    pub market: MarketInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PremiumToken {
    pub mint: Pubkey,
    pub token_program: Pubkey,
}

#[derive(Debug, Clone)]
pub struct PremiumBalance {
    pub token: PremiumToken,
    pub address: Pubkey,
    /// None means the ATA does not exist, not a failed read.
    pub amount: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct MakerSnapshot {
    pub positions: Vec<PositionSnapshot>,
    pub premium_balances: Vec<PremiumBalance>,
    pub missing_positions: Vec<Pubkey>,
}

impl PositionSnapshot {
    pub fn settlement_token(&self) -> PremiumToken {
        match self.position.position_type {
            crate::PositionType::CoveredCall => PremiumToken {
                mint: self.market.quote_mint,
                token_program: self.market.quote_token_program_id,
            },
            crate::PositionType::CashSecuredPut => PremiumToken {
                mint: self.market.underlying_mint,
                token_program: self.market.underlying_token_program_id,
            },
        }
    }

    pub fn required_settlement(&self) -> Result<u64, ChainError> {
        super::settlement::required_settlement(
            self.position.position_type.into(),
            self.position.strike.value(),
            self.position.quantity.value(),
            self.market.quote_decimals.value(),
            self.market.underlying_decimals.value(),
        )
    }

    /// Pure builder. Prepend explicit wrapping instructions when needed.
    pub fn fund_instructions(
        &self,
        program_id: &Pubkey,
        maker_owner: Pubkey,
        fee_payer: Pubkey,
        create_atas: bool,
    ) -> Result<Vec<Instruction>, ChainError> {
        if self.market_address != self.position.market_pda {
            return Err(ChainError::Identity("market"));
        }
        if self.position.maker_owner != maker_owner {
            return Err(ChainError::Identity("maker owner"));
        }
        if self.position.status != PositionStatus::Open
            || self.market.is_put
                != matches!(
                    self.position.position_type,
                    crate::PositionType::CashSecuredPut
                )
        {
            return Err(ChainError::InvalidAccountData);
        }
        Ok(ix::build_fund_position_ixs(
            program_id,
            &ix::FundPositionIxArgs {
                maker_owner,
                position_pda: self.address,
                market_pda: self.position.market_pda,
                position_type: self.position.position_type.into(),
                underlying_mint: self.market.underlying_mint,
                quote_mint: self.market.quote_mint,
                underlying_token_program_id: self.market.underlying_token_program_id,
                quote_token_program_id: self.market.quote_token_program_id,
                create_atas,
            },
            fee_payer,
        )?)
    }
}

pub fn decode_premium_balance(
    account: &Account,
    owner: &Pubkey,
    token: PremiumToken,
) -> Result<u64, ChainError> {
    if ![ix::TOKEN_PROGRAM_ID, ix::TOKEN_2022_PROGRAM_ID].contains(&token.token_program)
        || account.owner != token.token_program
        || account.data.len() < 165
        || account.data[..32] != token.mint.to_bytes()
        || account.data[32..64] != owner.to_bytes()
        || account.data[108] != 1
    {
        return Err(ChainError::InvalidAccountData);
    }
    Ok(u64::from_le_bytes(
        account.data[64..72]
            .try_into()
            .map_err(|_| ChainError::InvalidAccountData)?,
    ))
}

#[cfg(feature = "chain-rpc")]
mod rpc {
    use super::super::rpc::ChainClient;
    use super::*;
    use solana_rpc_client::api::filter::{Memcmp, RpcFilterType};

    pub fn maker_position_filters(owner: &Pubkey) -> Vec<RpcFilterType> {
        use super::super::accounts::{
            POSITION_DISCRIMINATOR, POSITION_LEN, POSITION_OFFSET_MAKER_OWNER,
        };
        vec![
            RpcFilterType::DataSize(POSITION_LEN as u64),
            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(0, &[POSITION_DISCRIMINATOR])),
            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                POSITION_OFFSET_MAKER_OWNER,
                owner.as_ref(),
            )),
        ]
    }

    impl ChainClient {
        /// Chunked reads preserve input order, including missing accounts.
        pub async fn fetch_accounts(
            &self,
            keys: &[Pubkey],
        ) -> Result<Vec<Option<Account>>, ChainError> {
            let mut accounts = Vec::with_capacity(keys.len());
            for chunk in keys.chunks(100) {
                let values = self
                    .rpc()
                    .get_multiple_accounts_with_commitment(chunk, self.rpc().commitment())
                    .await?
                    .value;
                if values.len() != chunk.len() {
                    return Err(ChainError::InvalidAccountData);
                }
                accounts.extend(values);
            }
            Ok(accounts)
        }

        /// Startup/manual recovery only. Includes Open and Funded positions; absence is not proof of closure.
        pub async fn maker_open_positions(
            &self,
            owner: &Pubkey,
        ) -> Result<Vec<(Pubkey, PositionInfo)>, ChainError> {
            let mut result = Vec::new();
            for (key, account) in self.program_accounts(maker_position_filters(owner)).await? {
                let position = PositionInfo::from_account(&account, &self.program_id())?;
                if position.maker_owner != *owner {
                    return Err(ChainError::Identity("maker owner"));
                }
                if matches!(
                    position.status,
                    PositionStatus::Open | PositionStatus::Funded
                ) {
                    result.push((key, position));
                }
            }
            Ok(result)
        }

        /// Two dependency rounds: positions + known premium ATAs, then unique markets.
        /// Chunks may observe different slots; this is not an atomic chain snapshot.
        pub async fn maker_snapshot(
            &self,
            owner: &Pubkey,
            positions: &[Pubkey],
            premiums: &[PremiumToken],
        ) -> Result<MakerSnapshot, ChainError> {
            if premiums.iter().any(|token| {
                ![ix::TOKEN_PROGRAM_ID, ix::TOKEN_2022_PROGRAM_ID].contains(&token.token_program)
            }) {
                return Err(ChainError::InvalidAccountData);
            }
            let maker = ix::maker_pda_with_program_id(&self.program_id(), owner).0;
            let mut keys = positions.to_vec();
            keys.extend(
                premiums.iter().map(|t| {
                    ix::derive_associated_token_address(&maker, &t.mint, &t.token_program)
                }),
            );
            keys.sort_unstable();
            keys.dedup();
            let accounts: BTreeMap<_, _> = keys
                .iter()
                .copied()
                .zip(self.fetch_accounts(&keys).await?)
                .collect();
            let mut decoded = Vec::new();
            let mut missing_positions = Vec::new();
            for key in positions {
                match &accounts[key] {
                    Some(account) => {
                        let position = PositionInfo::from_account(account, &self.program_id())?;
                        if position.maker_owner != *owner {
                            return Err(ChainError::Identity("maker owner"));
                        }
                        decoded.push((*key, position));
                    }
                    None => missing_positions.push(*key),
                }
            }
            let mut market_keys: Vec<_> = decoded.iter().map(|(_, p)| p.market_pda).collect();
            market_keys.sort_unstable();
            market_keys.dedup();
            let markets: BTreeMap<_, _> = market_keys
                .iter()
                .copied()
                .zip(self.fetch_accounts(&market_keys).await?)
                .map(|(key, account)| {
                    let account = account.ok_or(ChainError::MissingAccount(key))?;
                    Ok((key, MarketInfo::from_account(&account, &self.program_id())?))
                })
                .collect::<Result<_, ChainError>>()?;
            let premium_balances = premiums
                .iter()
                .map(|token| {
                    let address = ix::derive_associated_token_address(
                        &maker,
                        &token.mint,
                        &token.token_program,
                    );
                    let amount = accounts[&address]
                        .as_ref()
                        .map(|a| decode_premium_balance(a, &maker, *token))
                        .transpose()?;
                    Ok(PremiumBalance {
                        token: *token,
                        address,
                        amount,
                    })
                })
                .collect::<Result<_, ChainError>>()?;
            Ok(MakerSnapshot {
                positions: decoded
                    .into_iter()
                    .map(|(address, position)| PositionSnapshot {
                        market_address: position.market_pda,
                        market: markets[&position.market_pda].clone(),
                        address,
                        position,
                    })
                    .collect(),
                premium_balances,
                missing_positions,
            })
        }
    }
}
#[cfg(feature = "chain-rpc")]
pub use rpc::maker_position_filters;

#[cfg(all(test, feature = "chain-rpc"))]
#[path = "batch_tests.rs"]
mod tests;
