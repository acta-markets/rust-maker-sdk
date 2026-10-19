use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::manager::{VaultIx, VaultManager};
use super::{VaultChainError, pda};
use crate::chain::ix::{
    SYSTEM_PROGRAM_ID, derive_associated_token_address as ata, maker_pda_with_program_id,
};
use crate::orders::{OrderPreimageArgs, compute_order_id};
use crate::quote::market_pda;
use crate::types::PositionType;

pub const JUPITER_V6_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
/// Pool-backed mock router pinned by the devnet program build (devnet has no Jupiter).
pub const DEVNET_SWAP_ROUTER_ID: Pubkey =
    solana_sdk::pubkey!("BtHXpaAvhaKYk9pR4vNpuMZLTR8XeXnZ4iTy2Q1NzuCz");
pub const MAX_VAULT_SWAP_ROUTE_ACCOUNTS: usize = 64;
pub const MAX_VAULT_SWAP_ROUTE_DATA: usize = 1_024;

pub(crate) const INSTRUCTIONS_SYSVAR_ID: Pubkey =
    solana_sdk::pubkey!("Sysvar1nstructions1111111111111111111111111");

/// Terms of a vault option trade, as agreed in the RFQ and quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultTradeTerms {
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub expiry_ts: u64,
    pub is_put: bool,
    pub strike: u64,
    pub quantity: u64,
    pub price: u64,
    pub valid_until: u64,
    pub nonce: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultPositionAddresses {
    pub market: Pubkey,
    pub position: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_token_program: Pubkey,
    pub collateral_ata: Pubkey,
    pub config: Pubkey,
    pub protocol_fee_ata: Pubkey,
}

/// A position this vault holds, as its market and order id name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultPositionRef {
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub expiry_ts: u64,
    pub is_put: bool,
    pub order_id: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultOpenPosition {
    pub order_id: [u8; 32],
    pub addresses: VaultPositionAddresses,
    pub instruction: Instruction,
}

#[derive(Debug, Clone, Copy)]
struct Asset {
    mint: Pubkey,
    program: Pubkey,
    vault_ata: Pubkey,
}

#[derive(Debug, Clone, Copy)]
struct MarketLegs {
    underlying: Asset,
    quote: Asset,
}

impl MarketLegs {
    fn bind(
        manager: &VaultManager,
        underlying_mint: Pubkey,
        quote_mint: Pubkey,
    ) -> Result<Self, VaultChainError> {
        let main = Asset {
            mint: manager.args.main_mint,
            program: manager.args.main_token_program,
            vault_ata: manager.addresses.vault_main_ata,
        };
        let second = Asset {
            mint: manager.args.second_mint,
            program: manager.args.second_token_program,
            vault_ata: manager.addresses.vault_second_ata,
        };
        if main.mint == underlying_mint && second.mint == quote_mint {
            Ok(Self {
                underlying: main,
                quote: second,
            })
        } else if main.mint == quote_mint && second.mint == underlying_mint {
            Ok(Self {
                underlying: second,
                quote: main,
            })
        } else {
            Err(VaultChainError::InvalidInput(
                "market mints must be the vault's main and second mints",
            ))
        }
    }

    const fn collateral(self, is_put: bool) -> Asset {
        if is_put { self.quote } else { self.underlying }
    }

    const fn settlement(self, is_put: bool) -> Asset {
        if is_put { self.underlying } else { self.quote }
    }
}

impl VaultTradeTerms {
    fn validate(&self) -> Result<(), VaultChainError> {
        if self.expiry_ts == 0
            || self.strike == 0
            || self.quantity == 0
            || self.price == 0
            || self.valid_until == 0
        {
            return Err(VaultChainError::InvalidInput(
                "vault trade terms require non-zero expiry and economic fields",
            ));
        }
        if self.valid_until > self.expiry_ts {
            return Err(VaultChainError::InvalidInput(
                "vault trade quote cannot outlive its market",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn market(&self, program_id: &Pubkey) -> Pubkey {
        market_pda(
            program_id,
            &self.underlying_mint,
            &self.quote_mint,
            self.expiry_ts,
            self.is_put,
        )
    }

    /// Order id binding `maker` and `taker` to these terms.
    #[must_use]
    pub fn order_id(&self, program_id: &Pubkey, maker: &Pubkey, taker: &Pubkey) -> [u8; 32] {
        compute_order_id(&OrderPreimageArgs {
            chain_id: 0,
            program_id: program_id.to_bytes(),
            is_taker_buy: false,
            position_type: if self.is_put {
                PositionType::CashSecuredPut
            } else {
                PositionType::CoveredCall
            },
            market: self.market(program_id).to_bytes(),
            strike: self.strike,
            quantity: self.quantity,
            gross_price: self.price,
            valid_until: self.valid_until,
            maker: maker.to_bytes(),
            taker: taker.to_bytes(),
            nonce: self.nonce,
        })
    }

    fn args(&self, order_id: &[u8; 32]) -> Vec<u8> {
        let mut data = Vec::with_capacity(73);
        data.push(u8::from(self.is_put));
        for value in [
            self.strike,
            self.quantity,
            self.price,
            self.valid_until,
            self.nonce,
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(order_id);
        data
    }
}

/// A router route as the vault's swap instruction carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSwapRoute {
    /// Must equal the router pinned by the target program build.
    pub router: Pubkey,
    pub main_price_update: Pubkey,
    pub second_price_update: Pubkey,
    pub amount_in: u64,
    pub minimum_out: u64,
    pub valid_until: u64,
    pub route_accounts: Vec<AccountMeta>,
    pub route_data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultSwapKind {
    Rebalance { second_to_main: bool },
    Recovery,
    Closing,
}

impl VaultSwapKind {
    pub(crate) const fn fixed_accounts(self) -> usize {
        match self {
            Self::Rebalance { .. } => 18,
            Self::Recovery | Self::Closing => 15,
        }
    }
}

impl VaultManager {
    fn position_addresses(
        &self,
        legs: MarketLegs,
        terms: &VaultTradeTerms,
        order_id: &[u8; 32],
    ) -> VaultPositionAddresses {
        let program_id = &self.args.program_id;
        let position = pda::position(program_id, order_id).0;
        let config = pda::premium_config(program_id, &legs.quote.mint).0;
        let collateral = legs.collateral(terms.is_put);
        VaultPositionAddresses {
            market: terms.market(program_id),
            position,
            collateral_mint: collateral.mint,
            collateral_token_program: collateral.program,
            collateral_ata: ata(&position, &collateral.mint, &collateral.program),
            config,
            protocol_fee_ata: ata(&config, &legs.quote.mint, &legs.quote.program),
        }
    }

    fn market_tail(&self, legs: MarketLegs) -> [AccountMeta; 7] {
        [
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR_ID, false),
            AccountMeta::new_readonly(legs.underlying.mint, false),
            AccountMeta::new_readonly(legs.quote.mint, false),
            AccountMeta::new_readonly(legs.underlying.program, false),
            AccountMeta::new_readonly(legs.quote.program, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(self.addresses.global_config, false),
        ]
    }

    /// `VaultFundMakerPosition`: funds a held position's settlement leg (call: quote, put: underlying).
    pub fn fund_maker_position(
        &self,
        position: &VaultPositionRef,
    ) -> Result<Instruction, VaultChainError> {
        if position.expiry_ts == 0 || position.order_id == [0; 32] {
            return Err(VaultChainError::InvalidInput(
                "vault position requires a non-zero expiry and order id",
            ));
        }
        let legs = MarketLegs::bind(self, position.underlying_mint, position.quote_mint)?;
        let funding = legs.settlement(position.is_put);
        let program_id = &self.args.program_id;
        let position_pda = pda::position(program_id, &position.order_id).0;
        let market = market_pda(
            program_id,
            &position.underlying_mint,
            &position.quote_mint,
            position.expiry_ts,
            position.is_put,
        );
        let accounts = vec![
            AccountMeta::new_readonly(self.args.actor, true),
            AccountMeta::new_readonly(self.addresses.vault, false),
            AccountMeta::new_readonly(self.addresses.delegate, false),
            AccountMeta::new(position_pda, false),
            AccountMeta::new_readonly(market, false),
            AccountMeta::new(funding.vault_ata, false),
            AccountMeta::new(ata(&position_pda, &funding.mint, &funding.program), false),
            AccountMeta::new_readonly(funding.mint, false),
            AccountMeta::new_readonly(funding.program, false),
        ];
        Ok(self.instruction(VaultIx::VaultFundMakerPosition, accounts, &[]))
    }

    /// `OpenPositionForVaultAsTaker`: this vault writes and sells to `maker_owner`.
    pub fn open_position_as_taker(
        &self,
        maker_owner: Pubkey,
        terms: &VaultTradeTerms,
    ) -> Result<VaultOpenPosition, VaultChainError> {
        terms.validate()?;
        let legs = MarketLegs::bind(self, terms.underlying_mint, terms.quote_mint)?;
        let order_id = terms.order_id(&self.args.program_id, &maker_owner, &self.addresses.vault);
        let addresses = self.position_addresses(legs, terms, &order_id);
        let maker = maker_pda_with_program_id(&self.args.program_id, &maker_owner).0;
        let a = &self.addresses;
        let mut accounts = vec![
            AccountMeta::new(self.args.actor, true),
            AccountMeta::new(a.vault, false),
            AccountMeta::new_readonly(a.policy, false),
            AccountMeta::new_readonly(a.delegate, false),
            AccountMeta::new(a.vault_main_ata, false),
            AccountMeta::new(a.vault_second_ata, false),
            AccountMeta::new_readonly(maker, false),
            AccountMeta::new(ata(&maker, &legs.quote.mint, &legs.quote.program), false),
            AccountMeta::new(addresses.market, false),
            AccountMeta::new(addresses.position, false),
            AccountMeta::new(addresses.collateral_ata, false),
            AccountMeta::new_readonly(addresses.config, false),
            AccountMeta::new(addresses.protocol_fee_ata, false),
        ];
        accounts.extend(self.market_tail(legs));
        accounts.extend([
            AccountMeta::new_readonly(self.args.share_mint, false),
            AccountMeta::new_readonly(a.manager_stake_ata, false),
        ]);
        Ok(VaultOpenPosition {
            order_id,
            addresses,
            instruction: self.instruction(
                VaultIx::OpenPositionForVaultAsTaker,
                accounts,
                &terms.args(&order_id),
            ),
        })
    }

    /// `OpenPositionForVaultAsMaker`: `taker` writes against this vault's quote; the vault holds.
    pub fn open_position_as_maker(
        &self,
        taker: Pubkey,
        terms: &VaultTradeTerms,
    ) -> Result<VaultOpenPosition, VaultChainError> {
        terms.validate()?;
        let legs = MarketLegs::bind(self, terms.underlying_mint, terms.quote_mint)?;
        let order_id = terms.order_id(&self.args.program_id, &self.addresses.vault, &taker);
        let addresses = self.position_addresses(legs, terms, &order_id);
        let a = &self.addresses;
        let mut accounts = vec![
            AccountMeta::new(taker, true),
            AccountMeta::new(a.vault, false),
            AccountMeta::new_readonly(a.policy, false),
            AccountMeta::new_readonly(a.delegate, false),
            AccountMeta::new(legs.quote.vault_ata, false),
            AccountMeta::new(
                ata(&taker, &legs.underlying.mint, &legs.underlying.program),
                false,
            ),
            AccountMeta::new(ata(&taker, &legs.quote.mint, &legs.quote.program), false),
            AccountMeta::new(addresses.market, false),
            AccountMeta::new(addresses.position, false),
            AccountMeta::new(addresses.collateral_ata, false),
            AccountMeta::new_readonly(addresses.config, false),
            AccountMeta::new(addresses.protocol_fee_ata, false),
        ];
        accounts.extend(self.market_tail(legs));
        accounts.extend([
            AccountMeta::new_readonly(self.args.share_mint, false),
            AccountMeta::new_readonly(a.manager_stake_ata, false),
        ]);
        Ok(VaultOpenPosition {
            order_id,
            addresses,
            instruction: self.instruction(
                VaultIx::OpenPositionForVaultAsMaker,
                accounts,
                &terms.args(&order_id),
            ),
        })
    }

    /// `OpenPositionBetweenVaults`: this vault writes, `holder` quoted and holds.
    pub fn open_position_with_vault_holder(
        &self,
        holder: &Self,
        terms: &VaultTradeTerms,
    ) -> Result<VaultOpenPosition, VaultChainError> {
        terms.validate()?;
        let legs = MarketLegs::bind(self, terms.underlying_mint, terms.quote_mint)?;
        let holder_legs = MarketLegs::bind(holder, terms.underlying_mint, terms.quote_mint)?;
        if self.addresses.vault == holder.addresses.vault
            || self.args.program_id != holder.args.program_id
            || holder_legs.underlying.program != legs.underlying.program
            || holder_legs.quote.program != legs.quote.program
        {
            return Err(VaultChainError::InvalidInput(
                "vault-to-vault requires two distinct vaults on the same program and token pair",
            ));
        }
        let order_id = terms.order_id(
            &self.args.program_id,
            &holder.addresses.vault,
            &self.addresses.vault,
        );
        let addresses = self.position_addresses(legs, terms, &order_id);
        let (w, h) = (&self.addresses, &holder.addresses);
        let mut accounts = vec![
            AccountMeta::new(self.args.actor, true),
            AccountMeta::new(w.vault, false),
            AccountMeta::new_readonly(w.policy, false),
            AccountMeta::new_readonly(w.delegate, false),
            AccountMeta::new(w.vault_main_ata, false),
            AccountMeta::new(w.vault_second_ata, false),
            AccountMeta::new(h.vault, false),
            AccountMeta::new_readonly(h.policy, false),
            AccountMeta::new_readonly(h.delegate, false),
            AccountMeta::new(holder_legs.quote.vault_ata, false),
            AccountMeta::new(addresses.market, false),
            AccountMeta::new(addresses.position, false),
            AccountMeta::new(addresses.collateral_ata, false),
            AccountMeta::new_readonly(addresses.config, false),
            AccountMeta::new(addresses.protocol_fee_ata, false),
        ];
        accounts.extend(self.market_tail(legs));
        accounts.extend([
            AccountMeta::new_readonly(self.args.share_mint, false),
            AccountMeta::new_readonly(w.manager_stake_ata, false),
            AccountMeta::new_readonly(holder.args.share_mint, false),
            AccountMeta::new_readonly(h.manager_stake_ata, false),
        ]);
        Ok(VaultOpenPosition {
            order_id,
            addresses,
            instruction: self.instruction(
                VaultIx::OpenPositionBetweenVaults,
                accounts,
                &terms.args(&order_id),
            ),
        })
    }

    /// `VaultSpotRebalance`, `RecoveryConvertToMain` or `ClosingConvertToMain` with `route`.
    pub fn swap(
        &self,
        kind: VaultSwapKind,
        route: &VaultSwapRoute,
    ) -> Result<Instruction, VaultChainError> {
        self.validate_route(route)?;
        let route_data_len = u16::try_from(route.route_data.len())
            .map_err(|_| VaultChainError::InvalidInput("route data exceeds u16"))?;
        let a = &self.addresses;
        let args = &self.args;
        let oracle = [
            AccountMeta::new_readonly(args.main_mint, false),
            AccountMeta::new_readonly(args.second_mint, false),
            AccountMeta::new_readonly(a.main_oracle_source, false),
            AccountMeta::new_readonly(route.main_price_update, false),
            AccountMeta::new_readonly(a.second_oracle_source, false),
            AccountMeta::new_readonly(route.second_price_update, false),
            AccountMeta::new_readonly(args.main_token_program, false),
            AccountMeta::new_readonly(args.second_token_program, false),
            AccountMeta::new_readonly(route.router, false),
        ];
        let custody = [
            AccountMeta::new(a.vault_main_ata, false),
            AccountMeta::new(a.vault_second_ata, false),
        ];
        let actor = AccountMeta::new_readonly(args.actor, true);
        let vault = AccountMeta::new(a.vault, false);
        let policy = AccountMeta::new_readonly(a.policy, false);
        let mut accounts = Vec::with_capacity(kind.fixed_accounts() + route.route_accounts.len());
        let (ix, direction) = match kind {
            VaultSwapKind::Rebalance { second_to_main } => {
                accounts.extend([
                    actor,
                    vault,
                    policy,
                    AccountMeta::new_readonly(a.delegate, false),
                    AccountMeta::new_readonly(a.global_config, false),
                ]);
                accounts.extend(custody);
                accounts.extend(oracle);
                accounts.extend([
                    AccountMeta::new_readonly(args.share_mint, false),
                    AccountMeta::new_readonly(a.manager_stake_ata, false),
                ]);
                (VaultIx::VaultSpotRebalance, Some(u8::from(second_to_main)))
            }
            VaultSwapKind::Recovery => {
                accounts.extend([actor, vault, policy]);
                accounts.extend(custody);
                accounts.extend(oracle);
                accounts.push(AccountMeta::new_readonly(args.share_mint, false));
                (VaultIx::RecoveryConvertToMain, None)
            }
            VaultSwapKind::Closing => {
                accounts.extend([actor, vault, policy]);
                accounts.extend(custody);
                accounts.extend(oracle);
                accounts.push(AccountMeta::new_readonly(a.global_config, false));
                (VaultIx::ClosingConvertToMain, None)
            }
        };
        accounts.extend(route.route_accounts.iter().map(|meta| {
            AccountMeta {
                pubkey: meta.pubkey,
                is_signer: false,
                is_writable: route
                    .route_accounts
                    .iter()
                    .any(|other| other.pubkey == meta.pubkey && other.is_writable),
            }
        }));
        let mut data = Vec::with_capacity(28 + route.route_data.len());
        data.extend(direction);
        for value in [route.amount_in, route.minimum_out, route.valid_until] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&route_data_len.to_le_bytes());
        data.extend_from_slice(&route.route_data);
        Ok(self.instruction(ix, accounts, &data))
    }

    fn validate_route(&self, route: &VaultSwapRoute) -> Result<(), VaultChainError> {
        if route.amount_in == 0 || route.minimum_out == 0 || route.valid_until == 0 {
            return Err(VaultChainError::InvalidInput(
                "swap amounts and validity must be non-zero",
            ));
        }
        if route.route_accounts.is_empty()
            || route.route_accounts.len() > MAX_VAULT_SWAP_ROUTE_ACCOUNTS
        {
            return Err(VaultChainError::InvalidInput(
                "invalid vault swap route account count",
            ));
        }
        if route.route_data.is_empty() || route.route_data.len() > MAX_VAULT_SWAP_ROUTE_DATA {
            return Err(VaultChainError::InvalidInput(
                "invalid vault swap route data length",
            ));
        }
        let a = &self.addresses;
        if route
            .route_accounts
            .iter()
            .any(|meta| meta.is_signer && meta.pubkey != a.vault)
        {
            return Err(VaultChainError::InvalidInput(
                "only the vault PDA may sign the router CPI",
            ));
        }
        let contains = |key: &Pubkey| route.route_accounts.iter().any(|meta| meta.pubkey == *key);
        if !contains(&a.vault) || !contains(&a.vault_main_ata) || !contains(&a.vault_second_ata) {
            return Err(VaultChainError::InvalidInput(
                "route must contain vault, main ATA and second ATA",
            ));
        }
        Ok(())
    }
}
