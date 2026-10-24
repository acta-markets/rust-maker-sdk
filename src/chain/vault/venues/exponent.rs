//! Exponent 0.9.30 MarketThree and SPL stake-pool Generic SY.
use super::super::{VaultChainError, VaultSwapAmountOffset, VaultSwapHop};
use super::{Reader, invalid};
use crate::chain::ix::{TOKEN_PROGRAM_ID, derive_associated_token_address as ata};
use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

pub const CLMM: Pubkey = solana_sdk::pubkey!("XPC1MM4dYACDfykNuXYZ5una2DsMDWL24CrYubCvarC");
pub const GENERIC_SY: Pubkey = solana_sdk::pubkey!("XP1BRLn8eCYSygrd8er5P4GKdzqKbC3DLoSsS5UYVZy");
pub const CORE: Pubkey = solana_sdk::pubkey!("ExponentnaRg3CQbW6dqQNZKXp7gtZ9DGMp1cwC4HAS7");
pub const INDEX_ONE: u64 = 1_000_000_000_000;

#[derive(Debug, Clone)]
pub struct Market {
    pub address: Pubkey,
    pub lookup_table: Pubkey,
    pub pt: Pubkey,
    pub sy: Pubkey,
    pub vault: Pubkey,
    pub pt_escrow: Pubkey,
    pub sy_escrow: Pubkey,
    pub fee_sy: Pubkey,
    pub fee_pt: Pubkey,
    pub ticks: Pubkey,
    cpi: Vec<(u8, bool)>,
}

fn contexts(r: &mut Reader<'_>) -> Result<Vec<(u8, bool)>, VaultChainError> {
    let count = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..count {
        let index = r.byte()?;
        r.byte()?;
        let writable = r.byte()?;
        out.push((index, writable == 1));
    }
    Ok(out)
}
fn cpi(r: &mut Reader<'_>) -> Result<Vec<(u8, bool)>, VaultChainError> {
    let state = contexts(r)?;
    let deposit = contexts(r)?;
    let withdraw = contexts(r)?;
    for _ in 0..r.u32()? {
        contexts(r)?;
    }
    let position = contexts(r)?;
    Ok([state, position, deposit, withdraw].concat())
}
fn resolve(indices: &[(u8, bool)], table: &[Pubkey]) -> Result<Vec<AccountMeta>, VaultChainError> {
    let mut out: Vec<AccountMeta> = Vec::new();
    for &(index, writable) in indices {
        let key = *table
            .get(usize::from(index))
            .ok_or(invalid("Exponent ALT index missing"))?;
        if let Some(meta) = out.iter_mut().find(|m| m.pubkey == key) {
            meta.is_writable |= writable;
        } else {
            out.push(AccountMeta {
                pubkey: key,
                is_signer: false,
                is_writable: writable,
            });
        }
    }
    Ok(out)
}
impl Market {
    pub fn parse(address: Pubkey, data: &[u8]) -> Result<Self, VaultChainError> {
        if !data.starts_with(&[242, 240, 26, 15, 148, 186, 185, 205]) {
            return Err(invalid("not Exponent MarketThree"));
        }
        let mut r = Reader::new(data, 40);
        let lookup_table = r.key()?;
        let pt = r.key()?;
        let sy = r.key()?;
        r.take(32)?;
        let vault = r.key()?;
        let pt_escrow = r.key()?;
        let sy_escrow = r.key()?;
        r.take(32)?;
        let fee_sy = r.key()?;
        let fee_pt = r.key()?;
        if r.key()? != address {
            return Err(invalid("Exponent market identity mismatch"));
        }
        let ticks = r.key()?;
        r.take(2)?;
        if r.key()? != GENERIC_SY {
            return Err(invalid("unsupported SY program"));
        }
        let mut tail = Reader::new(data, 1588);
        tail.skip_vec(72)?;
        tail.take(4)?;
        tail.skip_vec(76)?;
        Ok(Self {
            address,
            lookup_table,
            pt,
            sy,
            vault,
            pt_escrow,
            sy_escrow,
            fee_sy,
            fee_pt,
            ticks,
            cpi: cpi(&mut tail)?,
        })
    }
    pub fn trade(
        &self,
        owner: Pubkey,
        amount: u64,
        buy: bool,
        table: &[Pubkey],
        patch: bool,
    ) -> Result<VaultSwapHop, VaultChainError> {
        let sy = ata(&owner, &self.sy, &TOKEN_PROGRAM_ID);
        let pt = ata(&owner, &self.pt, &TOKEN_PROGRAM_ID);
        let event = Pubkey::find_program_address(&[b"__event_authority"], &CLMM).0;
        let mut accounts = vec![
            w(owner),
            w(self.address),
            w(self.ticks),
            w(sy),
            w(pt),
            w(self.sy_escrow),
            w(self.pt_escrow),
            ro(self.lookup_table),
            ro(TOKEN_PROGRAM_ID),
            ro(GENERIC_SY),
            w(self.fee_sy),
            w(self.fee_pt),
            ro(event),
            ro(CLMM),
        ];
        accounts[0].is_signer = true;
        let mut remaining = resolve(&self.cpi, table)?;
        if !remaining.iter().any(|m| m.pubkey == self.vault) {
            remaining.push(ro(self.vault));
        }
        accounts.extend(remaining);
        let mut data = amount_data(3, amount);
        data.extend([u8::from(buy), 0, 0]);
        Ok(VaultSwapHop {
            program: CLMM,
            source: if buy { sy } else { pt },
            destination: if buy { pt } else { sy },
            accounts,
            data,
            amount_offset: patch.then_some(offset(1, 1)),
        })
    }
}

#[derive(Debug, Clone)]
pub struct GenericSy {
    pub address: Pubkey,
    pub mint: Pubkey,
    pub base: Pubkey,
    pub index: u64,
    pub interface_accounts: Vec<Pubkey>,
    emissions: Vec<Pubkey>,
}
impl GenericSy {
    pub fn parse(address: Pubkey, data: &[u8]) -> Result<Self, VaultChainError> {
        if !data.starts_with(&[254, 147, 136, 16, 163, 203, 98, 93]) {
            return Err(invalid("not Generic SY meta"));
        }
        let mint = Reader::new(data, 8).key()?;
        let mut r = Reader::new(data, 97);
        let index = r.u64()?;
        if index == 0 || r.take(24)?.iter().any(|b| *b != 0) {
            return Err(invalid("SY index is zero or exceeds u64"));
        }
        let base = r.key()?;
        if r.byte()? != 1 || r.byte()? != 0 {
            return Err(invalid("SY must be a hook-free SPL stake pool"));
        }
        r.take(33)?;
        r.skip_vec(1)?;
        r.skip_vec(1)?;
        let mut interface_accounts = Vec::new();
        for _ in 0..r.u32()? {
            interface_accounts.push(r.key()?);
        }
        if interface_accounts.is_empty() {
            return Err(invalid("SY has no stake pool"));
        }
        let mut emissions = Vec::new();
        for _ in 0..r.u32()? {
            let bytes = r.take(200)?;
            emissions.push(Reader::new(bytes, 128).key()?);
        }
        Ok(Self {
            address,
            mint,
            base,
            index,
            interface_accounts,
            emissions,
        })
    }
    pub fn mint_amount(&self, base: u64) -> Result<u64, VaultChainError> {
        let scaled = ((u128::from(base) + 1) * u128::from(self.index) - 1) / u128::from(INDEX_ONE);
        u64::try_from(scaled).map_err(|_| invalid("SY mint amount overflow"))
    }
    pub fn hop(
        &self,
        owner: Pubkey,
        base_program: Pubkey,
        amount: u64,
        mint: bool,
        patch: bool,
    ) -> Result<VaultSwapHop, VaultChainError> {
        let base = ata(&owner, &self.base, &base_program);
        let sy = ata(&owner, &self.mint, &TOKEN_PROGRAM_ID);
        let escrow = ata(&self.address, &self.base, &base_program);
        let mut accounts = if mint {
            vec![
                w(owner),
                w(self.address),
                w(self.mint),
                ro(self.base),
                w(base),
                w(escrow),
                w(sy),
                ro(base_program),
                ro(TOKEN_PROGRAM_ID),
            ]
        } else {
            vec![
                w(owner),
                w(self.address),
                w(base),
                w(escrow),
                w(sy),
                w(self.mint),
                ro(self.base),
                ro(base_program),
                ro(TOKEN_PROGRAM_ID),
            ]
        };
        accounts[0].is_signer = true;
        accounts.extend(self.interface_accounts.iter().copied().map(w));
        accounts.extend(self.emissions.iter().copied().map(ro));
        Ok(VaultSwapHop {
            program: GENERIC_SY,
            source: if mint { base } else { sy },
            destination: if mint { sy } else { base },
            accounts,
            data: amount_data(
                if mint { 1 } else { 2 },
                if patch {
                    0
                } else if mint {
                    self.mint_amount(amount)?
                } else {
                    amount
                },
            ),
            amount_offset: patch.then_some(if mint {
                offset(self.index, INDEX_ONE)
            } else {
                offset(1, 1)
            }),
        })
    }
}

#[derive(Debug, Clone)]
pub struct CoreVault {
    pub address: Pubkey,
    pub sy: Pubkey,
    pub yt: Pubkey,
    pub pt: Pubkey,
    pub escrow_sy: Pubkey,
    pub yield_position: Pubkey,
    pub lookup_table: Pubkey,
    pub maturity: u64,
    pub authority: Pubkey,
    cpi: Vec<(u8, bool)>,
}
impl CoreVault {
    pub fn parse(address: Pubkey, data: &[u8]) -> Result<Self, VaultChainError> {
        if !data.starts_with(&[211, 8, 232, 43, 2, 152, 117, 119]) {
            return Err(invalid("not Exponent core vault"));
        }
        let mut r = Reader::new(data, 8);
        if r.key()? != GENERIC_SY {
            return Err(invalid("unsupported core SY program"));
        }
        let sy = r.key()?;
        let yt = r.key()?;
        let pt = r.key()?;
        r.take(32)?;
        let escrow_sy = r.key()?;
        let yield_position = r.key()?;
        let lookup_table = r.key()?;
        let maturity = u64::from(r.u32()?) + u64::from(r.u32()?);
        r.take(32)?;
        let authority = r.key()?;
        let mut tail = Reader::new(data, 524);
        tail.skip_vec(170)?;
        Ok(Self {
            address,
            sy,
            yt,
            pt,
            escrow_sy,
            yield_position,
            lookup_table,
            maturity,
            authority,
            cpi: {
                let state = contexts(&mut tail)?;
                contexts(&mut tail)?;
                let withdraw = contexts(&mut tail)?;
                [state, withdraw].concat()
            },
        })
    }
    pub fn merge(
        &self,
        owner: Pubkey,
        amount: u64,
        now: u64,
        table: &[Pubkey],
    ) -> Result<VaultSwapHop, VaultChainError> {
        if now <= self.maturity {
            return Err(invalid("PT redemption requires maturity"));
        }
        let sy = ata(&owner, &self.sy, &TOKEN_PROGRAM_ID);
        let pt = ata(&owner, &self.pt, &TOKEN_PROGRAM_ID);
        let yt = ata(&owner, &self.yt, &TOKEN_PROGRAM_ID);
        let event = Pubkey::find_program_address(&[b"__event_authority"], &CORE).0;
        let mut accounts = vec![
            w(owner),
            w(self.authority),
            w(self.address),
            w(sy),
            w(self.escrow_sy),
            w(yt),
            w(pt),
            w(self.yt),
            w(self.pt),
            ro(TOKEN_PROGRAM_ID),
            ro(GENERIC_SY),
            ro(self.lookup_table),
            w(self.yield_position),
            ro(event),
            ro(CORE),
        ];
        accounts[0].is_signer = true;
        accounts.extend(resolve(&self.cpi, table)?);
        Ok(VaultSwapHop {
            program: CORE,
            source: pt,
            destination: sy,
            accounts,
            data: amount_data(5, amount),
            amount_offset: None,
        })
    }
}
fn w(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
fn ro(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}
fn offset(num: u64, den: u64) -> VaultSwapAmountOffset {
    VaultSwapAmountOffset {
        offset: 1,
        num,
        den,
    }
}
fn amount_data(tag: u8, amount: u64) -> Vec<u8> {
    let mut data = vec![tag];
    data.extend(amount.to_le_bytes());
    data
}

#[derive(Debug, Clone)]
pub struct StakePool {
    pub address: Pubkey,
    pub program: Pubkey,
    pub validator_list: Pubkey,
    pub reserve: Pubkey,
    pub mint: Pubkey,
    pub fee: Pubkey,
    pub token_program: Pubkey,
    pub epoch: u64,
}
impl StakePool {
    pub fn parse(address: Pubkey, program: Pubkey, data: &[u8]) -> Result<Self, VaultChainError> {
        if data.first() != Some(&1)
            || !super::super::served::STAKE_POOL_PROGRAM_IDS.contains(&program)
        {
            return Err(invalid("unsupported stake pool"));
        }
        let mut r = Reader::new(data, 98);
        let validator_list = r.key()?;
        let reserve = r.key()?;
        let mint = r.key()?;
        let fee = r.key()?;
        let token_program = r.key()?;
        r.take(16)?;
        let epoch = r.u64()?;
        Ok(Self {
            address,
            program,
            validator_list,
            reserve,
            mint,
            fee,
            token_program,
            epoch,
        })
    }
    pub fn update(&self) -> solana_sdk::instruction::Instruction {
        super::super::served::update_stake_pool_balance(
            self.program,
            [
                self.address,
                self.validator_list,
                self.reserve,
                self.fee,
                self.mint,
                self.token_program,
            ],
        )
    }
}
