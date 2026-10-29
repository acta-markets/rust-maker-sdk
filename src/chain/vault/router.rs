//! Pure PT routing; staged unstage parts are previews and must be rebuilt after confirmation.
pub use super::router_price::{OraclePrice, PriceClock, evaluate};
use super::venues::invalid;
use super::{
    venues::{
        SwapVenues,
        exponent::{CoreVault, GenericSy, Market},
        jupiter::Quote,
    },
    *,
};
use crate::chain::ix::ASSOCIATED_TOKEN_PROGRAM_ID;
use crate::types::{VaultLeg, VaultStagingStep};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{AddressLookupTableAccount, VersionedMessage, v0},
    pubkey::Pubkey,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Buy(u64),
    Sell(u64),
    Redeem(u64),
}
impl Intent {
    pub const fn amount(self) -> u64 {
        match self {
            Self::Buy(a) | Self::Sell(a) | Self::Redeem(a) => a,
        }
    }
    pub const fn buying(self) -> bool {
        matches!(self, Self::Buy(_))
    }
    pub fn backend_request(
        self,
        vault: Pubkey,
        slippage_bps: u16,
    ) -> Result<crate::ws::types::SwapQuoteData, VaultChainError> {
        if !self.buying() {
            return Err(invalid(
                "backend does not yet serve Exponent PT sell/redemption",
            ));
        }
        Ok(crate::ws::types::SwapQuoteData {
            request_id: uuid::Uuid::new_v4(),
            vault_pda: vault.into(),
            mode: crate::types::VaultSwapMode::Rebalance,
            direction: crate::types::VaultSwapDirection::MainToSecond,
            amount: self.amount().to_string(),
            slippage_bps,
        })
    }
}
#[derive(Debug, Clone)]
pub struct TxParts {
    pub instructions: Vec<Instruction>,
    pub tables: Vec<AddressLookupTableAccount>,
    pub kind: VaultSwapKind,
}
#[derive(Debug, Clone)]
pub enum Plan {
    Single(TxParts),
    Staged { stage: TxParts, unstage: TxParts },
}
impl Plan {
    pub fn first(&self) -> &TxParts {
        match self {
            Self::Single(tx) => tx,
            Self::Staged { stage, .. } => stage,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fit {
    pub account_locks: usize,
    pub bytes: usize,
}
impl Fit {
    pub const fn fits(self) -> bool {
        self.account_locks <= 64 && self.bytes <= 1232
    }
}
impl TxParts {
    pub fn compile(
        &self,
        payer: Pubkey,
        blockhash: Hash,
    ) -> Result<VersionedMessage, VaultChainError> {
        let message = v0::Message::try_compile(&payer, &self.instructions, &self.tables, blockhash)
            .map_err(|_| invalid("v0 transaction compilation failed"))?;
        Ok(VersionedMessage::V0(message))
    }
    pub fn fit(&self, payer: Pubkey) -> Result<Fit, VaultChainError> {
        let message = self.compile(payer, Hash::default())?;
        let VersionedMessage::V0(ref v0) = message else {
            return Err(invalid("expected v0 message"));
        };
        let account_locks = v0.account_keys.len()
            + v0.address_table_lookups
                .iter()
                .map(|l| l.writable_indexes.len() + l.readonly_indexes.len())
                .sum::<usize>();
        let signatures = usize::from(v0.header.num_required_signatures);
        Ok(Fit {
            account_locks,
            bytes: 1 + 64 * signatures + message.serialize().len(),
        })
    }
}

pub struct Router<'a> {
    pub manager: &'a VaultManager,
    pub account: &'a VaultAccount,
    pub main: &'a PriceRoute,
    pub second: &'a PriceRoute,
    pub staging: &'a PriceRoute,
    pub registry: &'a SwapVenues,
    pub market: &'a Market,
    pub sy: &'a GenericSy,
    pub core: &'a CoreVault,
    pub tables: &'a [AddressLookupTableAccount],
    pub shard: u16,
    pub now: u64,
    pub valid_until: u64,
    pub base_program: Pubkey,
    pub setup: Vec<Instruction>,
    pub prices: [OraclePrice; 3],
}
impl Router<'_> {
    fn table(&self, key: Pubkey) -> Result<&[Pubkey], VaultChainError> {
        self.tables
            .iter()
            .find(|t| t.key == key)
            .map(|t| t.addresses.as_slice())
            .ok_or(invalid("Exponent ALT missing"))
    }
    fn asset(&self) -> VaultStagingAsset {
        VaultStagingAsset {
            mint: self.sy.base,
            token_program: self.base_program,
        }
    }
    fn prices(&self, kind: VaultSwapKind) -> Result<Vec<AccountMeta>, VaultChainError> {
        if let VaultSwapKind::Staging { leg, .. } = kind {
            let route = if leg == VaultLeg::Main {
                self.main
            } else {
                self.second
            };
            let mut accounts = route.accounts(
                pda::price_route(&self.manager.args.program_id, &route.mint).0,
                self.shard,
            );
            accounts.extend(self.staging.accounts(
                pda::price_route(&self.manager.args.program_id, &self.staging.mint).0,
                self.shard,
            ));
            Ok(accounts)
        } else {
            self.manager
                .price_section(self.main, self.second, self.shard)
        }
    }
    fn parts(
        &self,
        kind: VaultSwapKind,
        amount: u64,
        minimum: u64,
        hops: Vec<VaultSwapHop>,
        quote: Option<&Quote>,
    ) -> Result<TxParts, VaultChainError> {
        if hops.iter().any(|h| !self.registry.allows(h)) {
            return Err(invalid("hop not allowed by SwapVenues"));
        }
        let mut instructions: Vec<_> = self
            .setup
            .iter()
            .filter(|ix| {
                ix.program_id != ASSOCIATED_TOKEN_PROGRAM_ID
                    || hops.iter().any(|hop| {
                        hop.accounts
                            .iter()
                            .any(|a| a.pubkey == ix.accounts[1].pubkey)
                    })
            })
            .cloned()
            .collect();
        instructions.push(self.manager.swap(
            kind.clone(),
            &VaultSwapPlan {
                price_section: self.prices(kind.clone())?,
                amount_in: amount,
                minimum_out: minimum,
                valid_until: self.valid_until,
                hops,
            },
        )?);
        let mut tables = self.tables.to_vec();
        for table in quote.into_iter().flat_map(|q| &q.tables) {
            if let Some(existing) = tables.iter().find(|t| t.key == table.key) {
                if existing.addresses != table.addresses {
                    return Err(invalid("conflicting lookup table contents"));
                }
            } else {
                tables.push(table.clone());
            }
        }
        Ok(TxParts {
            instructions,
            tables,
            kind,
        })
    }
    fn stage_kind(&self, leg: VaultLeg, step: VaultStagingStep, generation: u64) -> VaultSwapKind {
        VaultSwapKind::Staging {
            step,
            leg,
            asset: self.asset(),
            generation,
        }
    }
    pub(super) fn validate(&self, intent: Intent) -> Result<bool, VaultChainError> {
        self.manager.ensure_current(self.account)?;
        if self.valid_until <= self.now
            || (self.account.staging.open.is_none() && intent.amount() == 0)
        {
            return Err(invalid("invalid curator amount or deadline"));
        }
        if let Some(slot) = self.account.staging.open
            && slot.mint != self.sy.base
        {
            return Err(invalid("unsupported open staging mint"));
        }
        recovery_buy(self.account, intent, self.now, self.core.maturity)
    }
    pub fn plan(
        &self,
        intent: Intent,
        quote: Option<&Quote>,
        minimum_out: u64,
        minimum_staged: u64,
    ) -> Result<Plan, VaultChainError> {
        let buy = self.validate(intent)?;
        if minimum_out == 0 {
            return Err(invalid("invalid curator amount or deadline"));
        }
        let owner = self.manager.addresses.vault;
        if let Some(slot) = self.account.staging.open {
            let (leg, hops) = if buy {
                (
                    VaultLeg::Second,
                    vec![
                        self.sy
                            .hop(owner, self.base_program, slot.amount, true, false)?,
                        self.market.trade(
                            owner,
                            0,
                            true,
                            self.table(self.market.lookup_table)?,
                            true,
                        )?,
                    ],
                )
            } else {
                let quote = quote.ok_or(invalid("Jupiter quote required for unstage to main"))?;
                self.check_quote(quote, false, slot.amount)?;
                (VaultLeg::Main, vec![quote.hop(false)?])
            };
            let parts = self.parts(
                self.stage_kind(
                    leg,
                    VaultStagingStep::Unstage,
                    self.account.staging.generation,
                ),
                slot.amount,
                minimum_out,
                hops,
                quote,
            )?;
            return self.single_fitting(parts);
        }
        let quote = quote.ok_or(invalid("Jupiter quote required"))?;
        self.check_quote(
            quote,
            buy,
            if buy {
                intent.amount()
            } else {
                quote.request.amount
            },
        )?;
        let first = if buy {
            vec![quote.hop(false)?]
        } else {
            let hop = if matches!(intent, Intent::Redeem(_)) {
                self.core.merge(
                    owner,
                    intent.amount(),
                    self.now,
                    self.table(self.core.lookup_table)?,
                )?
            } else {
                self.market.trade(
                    owner,
                    intent.amount(),
                    false,
                    self.table(self.market.lookup_table)?,
                    false,
                )?
            };
            vec![hop, self.sy.hop(owner, self.base_program, 0, false, true)?]
        };
        let staged_amount = if buy {
            quote.output
        } else {
            quote.request.amount
        };
        let tail = |patch| -> Result<Vec<VaultSwapHop>, VaultChainError> {
            if buy {
                Ok(vec![
                    self.sy
                        .hop(owner, self.base_program, staged_amount, true, patch)?,
                    self.market.trade(
                        owner,
                        0,
                        true,
                        self.table(self.market.lookup_table)?,
                        true,
                    )?,
                ])
            } else {
                Ok(vec![quote.hop(patch)?])
            }
        };
        let single = self.parts(
            VaultSwapKind::Rebalance {
                second_to_main: !buy,
            },
            intent.amount(),
            minimum_out,
            [first.clone(), tail(true)?].concat(),
            Some(quote),
        );
        if let Ok(plan) = single.and_then(|tx| self.single_fitting(tx)) {
            return Ok(plan);
        }
        if minimum_staged == 0 {
            return Err(invalid("staging minimum must be nonzero"));
        }
        let generation = self
            .account
            .staging
            .generation
            .checked_add(1)
            .ok_or(invalid("staging generation overflow"))?;
        let stage = self.parts(
            self.stage_kind(
                if buy {
                    VaultLeg::Main
                } else {
                    VaultLeg::Second
                },
                VaultStagingStep::Stage,
                generation,
            ),
            intent.amount(),
            minimum_staged,
            first,
            Some(quote),
        )?;
        let unstage = self.parts(
            self.stage_kind(
                if buy {
                    VaultLeg::Second
                } else {
                    VaultLeg::Main
                },
                VaultStagingStep::Unstage,
                generation,
            ),
            staged_amount,
            minimum_out,
            tail(false)?,
            Some(quote),
        )?;
        if !stage.fit(self.manager.args.actor)?.fits()
            || !unstage.fit(self.manager.args.actor)?.fits()
        {
            return Err(invalid("curator transaction does not fit"));
        }
        Ok(Plan::Staged { stage, unstage })
    }
    fn single_fitting(&self, parts: TxParts) -> Result<Plan, VaultChainError> {
        if !parts.fit(self.manager.args.actor)?.fits() {
            return Err(invalid("curator transaction does not fit"));
        }
        Ok(Plan::Single(parts))
    }
    fn check_quote(&self, q: &Quote, buy: bool, amount: u64) -> Result<(), VaultChainError> {
        if q.request.amount != amount {
            return Err(invalid("Jupiter quote input mismatch"));
        }
        let (input, output) = if buy {
            (self.prices[0], self.prices[2])
        } else {
            (self.prices[2], self.prices[0])
        };
        input.check_quote(amount, output, q.output)
    }
}
pub(super) fn recovery_buy(
    account: &VaultAccount,
    intent: Intent,
    now: u64,
    maturity: u64,
) -> Result<bool, VaultChainError> {
    match account.staging.open.map(|slot| slot.from_leg) {
        Some(VaultLeg::Main) => Ok(now <= maturity
            && account.safety == VaultSafety::Normal
            && account.phase == VaultPhase::Active
            && now < account.active_expiry),
        Some(VaultLeg::Second) => Ok(false),
        None => match intent {
            Intent::Buy(_) if now >= maturity => Err(invalid("cannot buy matured PT")),
            Intent::Sell(_) if now > maturity => Err(invalid("use Redeem after maturity")),
            Intent::Redeem(_) if now <= maturity => Err(invalid("PT redemption requires maturity")),
            _ => Ok(intent.buying()),
        },
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FundingNeeds {
    pub main: u64,
    pub second: u64,
    pub cycle_expiry: u64,
    pub can_acquire_second: bool,
}
/// `price` is the current underlying/quote price at 1e9; ITM before expiry is provisional.
pub struct FundingMarket<'a> {
    pub address: Pubkey,
    pub market: &'a crate::chain::MarketInfo,
    pub price: u64,
}
pub fn funding_needs(
    vault: &VaultAccount,
    vault_address: Pubkey,
    positions: &[crate::chain::PositionInfo],
    markets: &[FundingMarket<'_>],
    now: u64,
) -> Result<FundingNeeds, VaultChainError> {
    let mut needs = FundingNeeds {
        cycle_expiry: vault.active_expiry,
        can_acquire_second: now < vault.active_expiry && vault.phase == VaultPhase::Active,
        ..FundingNeeds::default()
    };
    for position in positions {
        if position.maker_owner != vault_address
            || position.status != crate::chain::PositionStatus::Open
        {
            continue;
        }
        if position.terms.is_spread() {
            return Err(invalid("vaults cannot hold spreads"));
        }
        let item = markets
            .iter()
            .find(|m| m.address == position.market_pda)
            .ok_or(invalid("held position market missing"))?;
        let market = item.market;
        if market.expiry_ts != vault.active_expiry {
            continue;
        }
        if item.price == 0 {
            return Err(invalid("funding requires a nonzero price"));
        }
        let put = matches!(
            position.terms.position_type(),
            crate::types::PositionType::CashSecuredPut
        );
        let itm = if put {
            item.price < position.terms.strike().value()
        } else {
            item.price > position.terms.strike().value()
        };
        if !itm {
            continue;
        }
        let (mint, amount) = if put {
            (market.underlying_mint, position.quantity.value())
        } else {
            (
                market.quote_mint,
                crate::types::amounts::scaled_quote_amount(
                    position.terms.strike().value(),
                    position.quantity.value(),
                    market.quote_decimals.value(),
                    market.underlying_decimals.value(),
                )
                .ok_or(invalid("funding amount overflow"))?,
            )
        };
        let total = if mint == vault.main_mint {
            &mut needs.main
        } else if mint == vault.second_mint {
            &mut needs.second
        } else {
            return Err(invalid("funding mint outside vault pair"));
        };
        *total = total
            .checked_add(amount)
            .ok_or(invalid("funding sum overflow"))?;
    }
    Ok(needs)
}
