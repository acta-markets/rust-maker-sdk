use super::venues::invalid;
use super::{PriceBrick, PriceRoute, VaultChainError, pyth_price_feed_account};
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub struct PriceClock {
    pub now: u64,
    pub epoch: u64,
}
/// Off-chain midpoint estimate for quote screening; the contract remains the fixed-point authority.
#[derive(Debug, Clone, Copy)]
pub struct OraclePrice {
    pub mint: Pubkey,
    pub usd: f64,
    pub decimals: u8,
}
impl OraclePrice {
    pub fn convert(self, amount: u64, to: Self) -> Result<u64, VaultChainError> {
        let value = amount as f64 * self.usd / to.usd
            * 10f64.powi(i32::from(to.decimals) - i32::from(self.decimals));
        if !value.is_finite() || value < 1.0 || value >= u64::MAX as f64 {
            return Err(invalid("oracle conversion outside u64"));
        }
        Ok(value.floor() as u64)
    }
    pub fn check_quote(self, amount: u64, to: Self, output: u64) -> Result<(), VaultChainError> {
        let input_value = amount as f64 * self.usd / 10f64.powi(i32::from(self.decimals));
        let output_value = output as f64 * to.usd / 10f64.powi(i32::from(to.decimals));
        if !input_value.is_finite()
            || !output_value.is_finite()
            || output_value < input_value * 0.99
        {
            return Err(invalid("Jupiter quote exceeds one percent oracle loss"));
        }
        Ok(())
    }
}
fn field<const N: usize>(data: &[u8], at: usize) -> Result<[u8; N], VaultChainError> {
    data.get(at..at + N)
        .ok_or(invalid("truncated oracle source"))?
        .try_into()
        .map_err(|_| invalid("truncated oracle field"))
}
fn number(data: &[u8], at: usize, width: u8) -> Result<u128, VaultChainError> {
    match width {
        8 => Ok(u128::from(u64::from_le_bytes(field(data, at)?))),
        16 => Ok(u128::from_le_bytes(field(data, at)?)),
        _ => Err(invalid("unsupported oracle integer width")),
    }
}
pub fn evaluate(
    route: &PriceRoute,
    accounts: &BTreeMap<Pubkey, Account>,
    shard: u16,
    clock: PriceClock,
) -> Result<OraclePrice, VaultChainError> {
    let get = |key: &Pubkey| {
        accounts
            .get(key)
            .ok_or(invalid("oracle dependency missing"))
    };
    let mint = get(&route.mint)?;
    let decimals = *mint.data.get(44).ok_or(invalid("truncated mint"))?;
    if decimals > 18 {
        return Err(invalid("unsupported mint decimals"));
    }
    let mut mid = 1.0;
    let mut oldest = u64::MAX;
    let mut newest = 0;
    for brick in &route.bricks {
        match brick {
            PriceBrick::Pyth(p) => {
                let account = get(&pyth_price_feed_account(&p.feed_id, shard))?;
                let d = &account.data;
                let price = i64::from_le_bytes(field(d, 73)?);
                let exp = i32::from_le_bytes(field(d, 89)?);
                let published = i64::from_le_bytes(field(d, 93)?);
                if price <= 0 || published < 0 {
                    return Err(invalid("invalid Pyth price"));
                }
                let time = published as u64;
                if time > clock.now.saturating_add(5)
                    || clock.now.saturating_sub(time) > u64::from(p.max_age_secs)
                {
                    return Err(invalid("stale Pyth price"));
                }
                oldest = oldest.min(time);
                newest = newest.max(time);
                let scale = 10f64.powi(exp);
                mid *= price as f64 * scale;
            }
            PriceBrick::AccountRatio(r) => {
                let account = get(&r.account)?;
                let d = &account.data;
                let num = number(d, usize::from(r.numerator.at), r.numerator.width)?;
                let den = number(d, usize::from(r.denominator.at), r.denominator.width)?;
                let value = num as f64 / den as f64 * 10f64.powi(i32::from(r.scale_exp));
                mid *= value;
            }
            PriceBrick::MaturityDiscount(m) => {
                let years = m.maturity.saturating_sub(clock.now) as f64 / 31_536_000.0;
                mid *= (1.0 + f64::from(m.apy_mid_ppm) / 1e6).powf(-years);
            }
            PriceBrick::ClampMax { max } => {
                mid = mid.min(*max as f64 / 1e18);
            }
            PriceBrick::GrowthCap(_) => {}
        }
    }
    if oldest == u64::MAX
        || newest.saturating_sub(oldest) > u64::from(route.max_skew_secs)
        || !mid.is_finite()
        || mid <= 0.0
    {
        return Err(invalid("invalid oracle midpoint or skew"));
    }
    Ok(OraclePrice {
        mint: route.mint,
        usd: mid,
        decimals,
    })
}
