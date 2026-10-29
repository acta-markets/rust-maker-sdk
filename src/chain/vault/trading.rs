use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::manager::{VaultIx, VaultManager};
use super::{VaultChainError, pda};
use crate::chain::ix::{
    SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
    derive_associated_token_address as ata, maker_pda_with_program_id,
};
use crate::orders::{OrderPreimageArgs, build_physical_order_preimage};
use crate::quote::market_pda;
use crate::types::{PositionType, TakerSide, VaultLeg, VaultStagingStep};

/// CPI accounts per hop.
pub const MAX_VAULT_SWAP_ROUTE_ACCOUNTS: usize = 64;
/// Instruction account slots across all hops, duplicates included.
pub const MAX_VAULT_SWAP_PATH_ACCOUNTS: usize = 128;
/// Hop data bytes across all hops.
pub const MAX_VAULT_SWAP_ROUTE_DATA: usize = 1_024;
pub const MAX_VAULT_SWAP_HOPS: usize = 4;

pub(crate) const NO_AMOUNT_OFFSET: u16 = u16::MAX;

pub(crate) const INSTRUCTIONS_SYSVAR_ID: Pubkey =
    solana_sdk::pubkey!("Sysvar1nstructions1111111111111111111111111");

/// Terms of a vault option trade, as agreed in the RFQ and quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultTradeTerms {
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub expiry_ts: u64,
    pub is_put: bool,
    pub side: TakerSide,
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
        if self.side.is_taker_buy() {
            return Err(VaultChainError::InvalidInput("taker_buy_unsupported"));
        }
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
        crate::orders::hash_order_preimage(&build_physical_order_preimage(
            &OrderPreimageArgs {
                chain_id: 0,
                program_id: program_id.to_bytes(),
                side: self.side,
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
            },
            self.is_put,
        ))
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

/// One CPI into a registered swap venue; transit and scratch accounts must end where they started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSwapHop {
    pub program: Pubkey,
    /// Vault token account the hop spends: the input leg ATA, then the previous destination.
    pub source: Pubkey,
    /// Vault token account the hop fills: a transit vault ATA, the last hop the output leg.
    pub destination: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
    /// Not on the first hop: spends exactly what its source received earlier in this swap.
    pub amount_offset: Option<VaultSwapAmountOffset>,
}

/// Writes the largest `a` with `floor(a * den / num) == received` as u64 LE at `data[offset..]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultSwapAmountOffset {
    pub offset: u16,
    pub num: u64,
    pub den: u64,
}

/// A vault swap as its instruction carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSwapPlan {
    /// Main then second route with their accounts; see [`VaultManager::price_section`].
    pub price_section: Vec<AccountMeta>,
    /// Most the input leg may spend; the route may spend less and the rest stays in the leg.
    pub amount_in: u64,
    /// Least the output leg must gain. Drift is checked on the actual spend and gain.
    pub minimum_out: u64,
    pub valid_until: u64,
    /// Input leg to output leg, 1..=4 hops.
    pub hops: Vec<VaultSwapHop>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultSwapKind {
    Rebalance {
        second_to_main: bool,
    },
    Recovery,
    Closing,
    /// Stages SECOND during recovery, with MAIN custody and valuation before the hops.
    RecoveryStage {
        asset: VaultStagingAsset,
        generation: u64,
        main_price_section: Vec<AccountMeta>,
    },
    /// `VaultStagingSwap`; `generation` is `staged_generation + 1` to stage, else the open slot's.
    Staging {
        step: VaultStagingStep,
        leg: VaultLeg,
        asset: VaultStagingAsset,
        generation: u64,
    },
}

/// A mint with a price route the vault holds on its canonical ATA between staging halves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultStagingAsset {
    pub mint: Pubkey,
    pub token_program: Pubkey,
}

impl VaultStagingAsset {
    #[must_use]
    pub fn vault_ata(&self, vault: &Pubkey) -> Pubkey {
        ata(vault, &self.mint, &self.token_program)
    }
}

impl VaultSwapKind {
    pub(crate) const fn fixed_accounts(&self) -> usize {
        match self {
            Self::Rebalance { .. } | Self::Staging { .. } | Self::RecoveryStage { .. } => 14,
            Self::Recovery | Self::Closing => 11,
        }
    }

    /// Instruction data before the hops; it ends with `hop_count`.
    pub(crate) const fn header_len(&self) -> usize {
        match self {
            Self::Rebalance { .. } => 27,
            Self::Recovery | Self::Closing => 26,
            Self::Staging { .. } | Self::RecoveryStage { .. } => 36,
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
        keeper: Pubkey,
        maker_owner: Pubkey,
        terms: &VaultTradeTerms,
    ) -> Result<VaultOpenPosition, VaultChainError> {
        terms.validate()?;
        let legs = MarketLegs::bind(self, terms.underlying_mint, terms.quote_mint)?;
        let order_id = terms.order_id(&self.args.program_id, &maker_owner, &self.addresses.vault);
        let addresses = self.position_addresses(legs, terms, &order_id);
        Ok(self.open_as_taker(
            VaultIx::OpenPositionForVaultAsTaker,
            keeper,
            maker_owner,
            legs,
            order_id,
            addresses,
            &terms.args(&order_id),
        ))
    }

    /// `OpenPositionForVaultAsMaker`: `taker` writes against this vault's quote; the vault holds.
    pub fn open_position_as_maker(
        &self,
        keeper: Pubkey,
        taker: Pubkey,
        terms: &VaultTradeTerms,
    ) -> Result<VaultOpenPosition, VaultChainError> {
        terms.validate()?;
        let legs = MarketLegs::bind(self, terms.underlying_mint, terms.quote_mint)?;
        let order_id = terms.order_id(&self.args.program_id, &self.addresses.vault, &taker);
        let addresses = self.position_addresses(legs, terms, &order_id);
        Ok(self.open_as_maker(
            VaultIx::OpenPositionForVaultAsMaker,
            keeper,
            taker,
            legs,
            order_id,
            addresses,
            &terms.args(&order_id),
        ))
    }

    /// `OpenPositionBetweenVaults`: this vault writes, `holder` quoted and holds.
    pub fn open_position_with_vault_holder(
        &self,
        keeper: Pubkey,
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
        Ok(self.open_between_vaults(
            VaultIx::OpenPositionBetweenVaults,
            keeper,
            holder,
            holder_legs,
            legs,
            order_id,
            addresses,
            &terms.args(&order_id),
        ))
    }

    /// The swap instruction (rebalance, conversion or staging half) for `plan`.
    pub fn swap(
        &self,
        kind: VaultSwapKind,
        plan: &VaultSwapPlan,
    ) -> Result<Instruction, VaultChainError> {
        self.validate_swap(&kind, plan)?;
        if let VaultSwapKind::RecoveryStage {
            asset,
            generation,
            main_price_section,
        } = &kind
        {
            if main_price_section.is_empty()
                || main_price_section
                    .iter()
                    .any(|meta| meta.is_writable || meta.is_signer)
            {
                return Err(VaultChainError::InvalidInput(
                    "recovery main price section must be nonempty and read-only",
                ));
            }
            let mut instruction = self.swap(
                VaultSwapKind::Staging {
                    step: VaultStagingStep::Stage,
                    leg: VaultLeg::Second,
                    asset: *asset,
                    generation: *generation,
                },
                plan,
            )?;
            instruction.data[1] = 2;
            let tail = [
                AccountMeta::new(self.addresses.vault_main_ata, false),
                AccountMeta::new_readonly(self.args.main_token_program, false),
            ];
            instruction.accounts.splice(
                kind.fixed_accounts() + plan.price_section.len()
                    ..kind.fixed_accounts() + plan.price_section.len(),
                tail.into_iter().chain(main_price_section.iter().cloned()),
            );
            return Ok(instruction);
        }
        let a = &self.addresses;
        let args = &self.args;
        let actor = AccountMeta::new_readonly(args.actor, true);
        let vault = AccountMeta::new(a.vault, false);
        let policy = AccountMeta::new_readonly(a.policy, false);
        let custody = [
            AccountMeta::new(a.vault_main_ata, false),
            AccountMeta::new(a.vault_second_ata, false),
        ];
        let pair = [
            AccountMeta::new_readonly(args.main_mint, false),
            AccountMeta::new_readonly(args.second_mint, false),
            AccountMeta::new_readonly(args.main_token_program, false),
            AccountMeta::new_readonly(args.second_token_program, false),
        ];
        let mut accounts = Vec::with_capacity(
            kind.fixed_accounts()
                + plan.price_section.len()
                + plan
                    .hops
                    .iter()
                    .map(|hop| 3 + hop.accounts.len())
                    .sum::<usize>(),
        );
        let (ix, prefix) = match kind {
            VaultSwapKind::RecoveryStage { .. } => unreachable!("handled above"),
            VaultSwapKind::Rebalance { second_to_main } => {
                accounts.extend([
                    actor,
                    vault,
                    policy,
                    AccountMeta::new_readonly(a.delegate, false),
                    AccountMeta::new_readonly(a.global_config, false),
                ]);
                accounts.extend(custody);
                accounts.extend(pair);
                accounts.extend([
                    AccountMeta::new_readonly(args.share_mint, false),
                    AccountMeta::new_readonly(a.manager_stake_ata, false),
                ]);
                (
                    VaultIx::VaultSpotRebalance,
                    [Some(u8::from(second_to_main)), None],
                )
            }
            VaultSwapKind::Recovery => {
                accounts.extend([actor, vault, policy]);
                accounts.extend(custody);
                accounts.extend(pair);
                accounts.push(AccountMeta::new_readonly(args.share_mint, false));
                (VaultIx::RecoveryConvertToMain, [None, None])
            }
            VaultSwapKind::Closing => {
                accounts.extend([actor, vault, policy]);
                accounts.extend(custody);
                accounts.extend(pair);
                accounts.push(AccountMeta::new_readonly(a.global_config, false));
                (VaultIx::ClosingConvertToMain, [None, None])
            }
            VaultSwapKind::Staging {
                step, leg, asset, ..
            } => {
                let (leg_ata, leg_mint, leg_program, leg) = match leg {
                    VaultLeg::Main => {
                        (a.vault_main_ata, args.main_mint, args.main_token_program, 0)
                    }
                    VaultLeg::Second => (
                        a.vault_second_ata,
                        args.second_mint,
                        args.second_token_program,
                        1,
                    ),
                };
                let step = match step {
                    VaultStagingStep::Stage => 0,
                    VaultStagingStep::Unstage => 1,
                };
                accounts.extend([
                    actor,
                    vault,
                    policy,
                    AccountMeta::new_readonly(a.delegate, false),
                    AccountMeta::new_readonly(a.global_config, false),
                    AccountMeta::new(leg_ata, false),
                    AccountMeta::new_readonly(leg_mint, false),
                    AccountMeta::new_readonly(leg_program, false),
                    AccountMeta::new(asset.vault_ata(&a.vault), false),
                    AccountMeta::new_readonly(asset.mint, false),
                    AccountMeta::new_readonly(asset.token_program, false),
                    AccountMeta::new_readonly(args.share_mint, false),
                    AccountMeta::new_readonly(a.manager_stake_ata, false),
                ]);
                (VaultIx::VaultStagingSwap, [Some(step), Some(leg)])
            }
        };
        accounts.push(AccountMeta::new_readonly(a.swap_venues, false));
        accounts.extend_from_slice(&plan.price_section);
        let mut data = Vec::with_capacity(
            kind.header_len()
                + plan
                    .hops
                    .iter()
                    .map(|hop| {
                        let scale = if hop.amount_offset.is_some() { 16 } else { 0 };
                        5 + scale + hop.data.len()
                    })
                    .sum::<usize>(),
        );
        data.extend(prefix.into_iter().flatten());
        for value in [plan.amount_in, plan.minimum_out, plan.valid_until] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        if let VaultSwapKind::Staging { generation, .. } = kind {
            data.extend_from_slice(&generation.to_le_bytes());
        }
        // Lengths are bounded by `validate_swap`.
        data.push(plan.hops.len() as u8);
        for hop in &plan.hops {
            data.push(hop.accounts.len() as u8);
            data.extend_from_slice(&(hop.data.len() as u16).to_le_bytes());
            match hop.amount_offset {
                Some(amount) => {
                    data.extend_from_slice(&amount.offset.to_le_bytes());
                    data.extend_from_slice(&amount.num.to_le_bytes());
                    data.extend_from_slice(&amount.den.to_le_bytes());
                }
                None => data.extend_from_slice(&NO_AMOUNT_OFFSET.to_le_bytes()),
            }
            data.extend_from_slice(&hop.data);
            accounts.extend([
                AccountMeta::new_readonly(hop.program, false),
                AccountMeta::new(hop.source, false),
                AccountMeta::new(hop.destination, false),
            ]);
            accounts.extend(hop.accounts.iter().map(|meta| AccountMeta {
                is_signer: false,
                ..meta.clone()
            }));
        }
        Ok(self.instruction(ix, accounts, &data))
    }

    fn validate_swap(
        &self,
        kind: &VaultSwapKind,
        plan: &VaultSwapPlan,
    ) -> Result<(), VaultChainError> {
        let invalid = |reason| Err(VaultChainError::InvalidInput(reason));
        if plan.amount_in == 0 || plan.minimum_out == 0 || plan.valid_until == 0 {
            return invalid("swap amounts and validity must be non-zero");
        }
        let hops = &plan.hops;
        if hops.is_empty() || hops.len() > MAX_VAULT_SWAP_HOPS {
            return invalid("invalid vault swap hop count");
        }
        let accounts: usize = hops.iter().map(|hop| hop.accounts.len() + 3).sum();
        let data: usize = hops.iter().map(|hop| hop.data.len()).sum();
        if accounts > MAX_VAULT_SWAP_PATH_ACCOUNTS || data > MAX_VAULT_SWAP_ROUTE_DATA {
            return invalid("vault swap hops exceed the account or data cap");
        }
        let a = &self.addresses;
        let (input, output) = &self.swap_legs(kind)?;
        let mut source = input;
        for (index, hop) in hops.iter().enumerate() {
            let last = index + 1 == hops.len();
            if hop.accounts.is_empty() || hop.accounts.len() > MAX_VAULT_SWAP_ROUTE_ACCOUNTS {
                return invalid("invalid vault swap hop account count");
            }
            if hop.data.is_empty() {
                return invalid("vault swap hop needs data");
            }
            if let Some(amount) = hop.amount_offset
                && (index == 0
                    || amount.offset == NO_AMOUNT_OFFSET
                    || usize::from(amount.offset) + 8 > hop.data.len()
                    || amount.num == 0
                    || amount.den == 0)
            {
                return invalid(
                    "amount_offset needs a transit source, 8 bytes inside the data and a nonzero scale",
                );
            }
            if hop.source != *source
                || (last && hop.destination != *output)
                || (!last && (hop.destination == *input || hop.destination == *output))
            {
                return invalid("vault swap hops must chain input leg, transit ATAs, output leg");
            }
            if hop
                .accounts
                .iter()
                .any(|meta| meta.is_signer && meta.pubkey != a.vault)
            {
                return invalid("only the vault PDA may sign a hop CPI");
            }
            if hop.accounts.iter().any(|meta| {
                (meta.pubkey == *input || meta.pubkey == *output)
                    && meta.pubkey != hop.source
                    && meta.pubkey != hop.destination
            }) {
                return invalid("a leg ATA may appear only as its hop's source or destination");
            }
            let contains = |key: &Pubkey| hop.accounts.iter().any(|meta| meta.pubkey == *key);
            if !contains(&a.vault) || !contains(&hop.source) || !contains(&hop.destination) {
                return invalid("hop must contain the vault, its source and its destination");
            }
            source = &hop.destination;
        }
        Ok(())
    }

    /// Input and output token accounts of `kind`.
    fn swap_legs(&self, kind: &VaultSwapKind) -> Result<(Pubkey, Pubkey), VaultChainError> {
        let a = &self.addresses;
        let (main, second) = (a.vault_main_ata, a.vault_second_ata);
        if let VaultSwapKind::RecoveryStage {
            asset, generation, ..
        } = kind
        {
            return self.swap_legs(&VaultSwapKind::Staging {
                step: VaultStagingStep::Stage,
                leg: VaultLeg::Second,
                asset: *asset,
                generation: *generation,
            });
        }
        let VaultSwapKind::Staging {
            step, leg, asset, ..
        } = kind
        else {
            return Ok(match kind {
                VaultSwapKind::Rebalance {
                    second_to_main: false,
                } => (main, second),
                _ => (second, main),
            });
        };
        let args = &self.args;
        if [args.main_mint, args.second_mint, args.share_mint].contains(&asset.mint) {
            return Err(VaultChainError::InvalidInput(
                "the staging mint must not be a vault mint",
            ));
        }
        if asset.token_program != TOKEN_PROGRAM_ID && asset.token_program != TOKEN_2022_PROGRAM_ID {
            return Err(VaultChainError::InvalidInput(
                "staging token program is not SPL Token",
            ));
        }
        let leg = match leg {
            VaultLeg::Main => main,
            VaultLeg::Second => second,
        };
        let staging = asset.vault_ata(&a.vault);
        match step {
            VaultStagingStep::Stage => Ok((leg, staging)),
            VaultStagingStep::Unstage => Ok((staging, leg)),
        }
    }
}

/// Terms of a vault cash spread, as signed in the spread order layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultSpreadTerms {
    pub underlying_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub expiry_ts: u64,
    pub kind: crate::spreads::SpreadType,
    pub side: TakerSide,
    pub lower_strike: u64,
    pub upper_strike: u64,
    pub quantity: u64,
    pub price: u64,
    pub valid_until: u64,
    pub nonce: u64,
}

/// Terms for a vault open: a physical option or a cash spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultSaleTerms {
    Physical(VaultTradeTerms),
    Spread(VaultSpreadTerms),
}

impl VaultSpreadTerms {
    fn validate(&self) -> Result<(), VaultChainError> {
        if self.side.is_taker_buy() {
            return Err(VaultChainError::InvalidInput("taker_buy_unsupported"));
        }
        crate::spreads::validate_premium(
            self.kind,
            self.lower_strike,
            self.upper_strike,
            self.quantity,
            self.price,
        )
        .map_err(|_| {
            VaultChainError::InvalidInput("vault spread requires ordered strikes, positive call lower strike, and 0 < price < width")
        })?;
        if self.expiry_ts == 0 || self.valid_until == 0 || self.valid_until > self.expiry_ts {
            return Err(VaultChainError::InvalidInput(
                "vault spread quote must be valid before its market expiry",
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
            self.kind == crate::spreads::SpreadType::PutSpread,
        )
    }

    /// Spread-layout order id binding `maker` and `taker` to these terms.
    pub fn order_id(
        &self,
        program_id: &Pubkey,
        maker: &Pubkey,
        taker: &Pubkey,
    ) -> Result<[u8; 32], VaultChainError> {
        crate::orders::compute_spread_order_id(&crate::orders::SpreadOrderPreimageArgs {
            chain_id: 0,
            program_id: program_id.to_bytes(),
            side: self.side,
            position_type: self.kind,
            market: self.market(program_id).to_bytes(),
            lower_strike: self.lower_strike,
            upper_strike: self.upper_strike,
            quantity: self.quantity,
            gross_price: self.price,
            valid_until: self.valid_until,
            maker: maker.to_bytes(),
            taker: taker.to_bytes(),
            nonce: self.nonce,
        })
        .map_err(|_| VaultChainError::InvalidInput("invalid vault spread order"))
    }

    fn args(&self, order_id: &[u8; 32]) -> Vec<u8> {
        let mut data = Vec::with_capacity(81);
        data.push(self.kind as u8);
        for value in [
            self.lower_strike,
            self.quantity,
            self.price,
            self.valid_until,
            self.nonce,
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(order_id);
        data.extend_from_slice(&self.upper_strike.to_le_bytes());
        data
    }
}

impl VaultManager {
    /// Spread collateral is quote atoms escrowed in the position's quote ATA.
    fn spread_addresses(
        &self,
        legs: MarketLegs,
        terms: &VaultSpreadTerms,
        order_id: &[u8; 32],
    ) -> VaultPositionAddresses {
        let program_id = &self.args.program_id;
        let position = pda::position(program_id, order_id).0;
        let config = pda::premium_config(program_id, &legs.quote.mint).0;
        VaultPositionAddresses {
            market: terms.market(program_id),
            position,
            collateral_mint: legs.quote.mint,
            collateral_token_program: legs.quote.program,
            collateral_ata: ata(&position, &legs.quote.mint, &legs.quote.program),
            config,
            protocol_fee_ata: ata(&config, &legs.quote.mint, &legs.quote.program),
        }
    }

    /// `OpenSpreadForVaultAsTaker`: this vault sells; `C` leaves its quote leg.
    pub fn open_spread_as_taker(
        &self,
        keeper: Pubkey,
        maker_owner: Pubkey,
        terms: &VaultSpreadTerms,
    ) -> Result<VaultOpenPosition, VaultChainError> {
        terms.validate()?;
        let legs = MarketLegs::bind(self, terms.underlying_mint, terms.quote_mint)?;
        let order_id =
            terms.order_id(&self.args.program_id, &maker_owner, &self.addresses.vault)?;
        let addresses = self.spread_addresses(legs, terms, &order_id);
        Ok(self.open_as_taker(
            VaultIx::OpenSpreadForVaultAsTaker,
            keeper,
            maker_owner,
            legs,
            order_id,
            addresses,
            &terms.args(&order_id),
        ))
    }

    /// `OpenSpreadForVaultAsMaker`: this vault buys; `G` leaves its quote leg.
    pub fn open_spread_as_maker(
        &self,
        keeper: Pubkey,
        taker: Pubkey,
        terms: &VaultSpreadTerms,
    ) -> Result<VaultOpenPosition, VaultChainError> {
        terms.validate()?;
        let legs = MarketLegs::bind(self, terms.underlying_mint, terms.quote_mint)?;
        let order_id = terms.order_id(&self.args.program_id, &self.addresses.vault, &taker)?;
        let addresses = self.spread_addresses(legs, terms, &order_id);
        Ok(self.open_as_maker(
            VaultIx::OpenSpreadForVaultAsMaker,
            keeper,
            taker,
            legs,
            order_id,
            addresses,
            &terms.args(&order_id),
        ))
    }

    /// `OpenSpreadBetweenVaults`: this vault sells the spread to `holder`.
    pub fn open_spread_with_vault_holder(
        &self,
        keeper: Pubkey,
        holder: &Self,
        terms: &VaultSpreadTerms,
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
        )?;
        let addresses = self.spread_addresses(legs, terms, &order_id);
        Ok(self.open_between_vaults(
            VaultIx::OpenSpreadBetweenVaults,
            keeper,
            holder,
            holder_legs,
            legs,
            order_id,
            addresses,
            &terms.args(&order_id),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn open_as_taker(
        &self,
        ix: VaultIx,
        keeper: Pubkey,
        maker_owner: Pubkey,
        legs: MarketLegs,
        order_id: [u8; 32],
        addresses: VaultPositionAddresses,
        args: &[u8],
    ) -> VaultOpenPosition {
        let maker = maker_pda_with_program_id(&self.args.program_id, &maker_owner).0;
        let a = &self.addresses;
        let mut accounts = vec![
            AccountMeta::new(keeper, true),
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
        VaultOpenPosition {
            order_id,
            addresses,
            instruction: self.instruction(ix, accounts, args),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn open_as_maker(
        &self,
        ix: VaultIx,
        keeper: Pubkey,
        taker: Pubkey,
        legs: MarketLegs,
        order_id: [u8; 32],
        addresses: VaultPositionAddresses,
        args: &[u8],
    ) -> VaultOpenPosition {
        let a = &self.addresses;
        let mut accounts = vec![
            AccountMeta::new(keeper, true),
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
        VaultOpenPosition {
            order_id,
            addresses,
            instruction: self.instruction(ix, accounts, args),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn open_between_vaults(
        &self,
        ix: VaultIx,
        keeper: Pubkey,
        holder: &Self,
        holder_legs: MarketLegs,
        legs: MarketLegs,
        order_id: [u8; 32],
        addresses: VaultPositionAddresses,
        args: &[u8],
    ) -> VaultOpenPosition {
        let (w, h) = (&self.addresses, &holder.addresses);
        let mut accounts = vec![
            AccountMeta::new(keeper, true),
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
        VaultOpenPosition {
            order_id,
            addresses,
            instruction: self.instruction(ix, accounts, args),
        }
    }
}
