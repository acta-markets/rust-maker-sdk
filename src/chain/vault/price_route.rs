use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

use super::VaultChainError;
use super::account::{header, pubkey};

pub const PYTH_PUSH_ORACLE_ID: Pubkey =
    solana_sdk::pubkey!("pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT");
pub const MAX_PRICE_BRICKS: usize = 6;
pub const MAX_PRICE_ROUTE_BYTES: usize = 640;
pub const MAX_RATIO_DISCRIMINATOR_LEN: usize = 8;

const PRICE_ROUTE_DISCRIMINATOR: u8 = 15;
const PRICE_ROUTE_VERSION: u8 = 1;
const BRICKS_OFFSET: usize = 48;

#[must_use]
pub fn pyth_price_feed_account(feed_id: &[u8; 32], shard: u16) -> Pubkey {
    Pubkey::find_program_address(&[&shard.to_le_bytes(), feed_id], &PYTH_PUSH_ORACLE_ID).0
}

/// One factor of a price route; the route is their product, left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PriceBrick {
    Pyth(PythBrick),
    AccountRatio(AccountRatioBrick),
    MaturityDiscount(MaturityDiscountBrick),
    /// Lowers every bound of the running product to at most `max` (1e18).
    ClampMax {
        max: u128,
    },
    GrowthCap(GrowthCapBrick),
}

/// A Pyth `PriceUpdateV2` of `feed_id`, read from the push-oracle account on the served shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PythBrick {
    pub feed_id: [u8; 32],
    pub max_age_secs: u32,
    pub min_band_ppm: u32,
}

/// `numerator / denominator * 10^scale_exp` of a pinned `account`, read with its `program_data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRatioBrick {
    pub account: Pubkey,
    pub owner: Pubkey,
    pub program_data: Pubkey,
    pub deploy_slot: u64,
    pub discriminator: Vec<u8>,
    pub min_len: u16,
    pub mint_at: Option<u16>,
    pub numerator: RatioField,
    pub denominator: RatioField,
    pub scale_exp: i8,
    pub min_denominator: u64,
    pub min: u128,
    pub max: u128,
    pub freshness: Option<RatioFreshness>,
}

/// An unsigned LE integer of `width` bytes at `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatioField {
    pub at: u16,
    pub width: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatioFreshness {
    pub unit: FreshnessUnit,
    pub at: u16,
    pub max_lag: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessUnit {
    Epoch,
    Slot,
    UnixSecond,
}

/// Multiplies by `(1 + apy)^-years` to `maturity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaturityDiscountBrick {
    pub maturity: u64,
    pub apy_low_ppm: u32,
    pub apy_mid_ppm: u32,
    pub apy_high_ppm: u32,
    pub maturity_source: Option<MaturitySource>,
}

/// `maturity` equals the sum of two `u32` fields of the account of brick `brick`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaturitySource {
    pub brick: u8,
    pub start_at: u16,
    pub duration_at: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrowthCapBrick {
    pub anchor_rate: u128,
    pub anchor_ts: u64,
    pub max_growth_ppm: u32,
    pub max_anchor_age_secs: u32,
}

impl PriceBrick {
    fn decode(r: &mut Reader<'_>) -> Result<Self, VaultChainError> {
        Ok(match r.u8()? {
            0 => Self::Pyth(PythBrick {
                feed_id: r.array()?,
                max_age_secs: r.u32()?,
                min_band_ppm: r.u32()?,
            }),
            1 => Self::AccountRatio(AccountRatioBrick {
                account: r.pubkey()?,
                owner: r.pubkey()?,
                program_data: r.pubkey()?,
                deploy_slot: r.u64()?,
                discriminator: {
                    let len = usize::from(r.u8()?);
                    if len > MAX_RATIO_DISCRIMINATOR_LEN {
                        return Err(VaultChainError::PriceRoute(
                            "ratio discriminator exceeds 8 bytes",
                        ));
                    }
                    r.take(len)?.to_vec()
                },
                min_len: r.u16()?,
                mint_at: r.option(Reader::u16)?,
                numerator: r.field()?,
                denominator: r.field()?,
                scale_exp: i8::from_le_bytes(r.array()?),
                min_denominator: r.u64()?,
                min: r.u128()?,
                max: r.u128()?,
                freshness: r.option(|r| {
                    Ok(RatioFreshness {
                        unit: match r.u8()? {
                            0 => FreshnessUnit::Epoch,
                            1 => FreshnessUnit::Slot,
                            2 => FreshnessUnit::UnixSecond,
                            _ => return Err(VaultChainError::PriceRoute("unknown freshness unit")),
                        },
                        at: r.u16()?,
                        max_lag: r.u32()?,
                    })
                })?,
            }),
            2 => Self::MaturityDiscount(MaturityDiscountBrick {
                maturity: r.u64()?,
                apy_low_ppm: r.u32()?,
                apy_mid_ppm: r.u32()?,
                apy_high_ppm: r.u32()?,
                maturity_source: r.option(|r| {
                    Ok(MaturitySource {
                        brick: r.u8()?,
                        start_at: r.u16()?,
                        duration_at: r.u16()?,
                    })
                })?,
            }),
            3 => Self::ClampMax { max: r.u128()? },
            4 => Self::GrowthCap(GrowthCapBrick {
                anchor_rate: r.u128()?,
                anchor_ts: r.u64()?,
                max_growth_ppm: r.u32()?,
                max_anchor_age_secs: r.u32()?,
            }),
            _ => return Err(VaultChainError::PriceRoute("unknown brick kind")),
        })
    }

    #[must_use]
    pub const fn account_count(&self) -> usize {
        match self {
            Self::Pyth(_) => 1,
            Self::AccountRatio(_) => 2,
            Self::MaturityDiscount(_) | Self::ClampMax { .. } | Self::GrowthCap(_) => 0,
        }
    }
}

/// `PriceRoute` account (`["price_route", mint]`): how the program prices `mint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceRoute {
    pub mint: Pubkey,
    pub max_skew_secs: u32,
    pub bricks: Vec<PriceBrick>,
    pub updated_slot: u64,
}

impl PriceRoute {
    pub const LEN: usize = 696;

    /// [`Self::parse`] for an account that must be owned by `program_id`.
    pub fn from_account(
        account: &solana_sdk::account::Account,
        program_id: &Pubkey,
    ) -> Result<Self, VaultChainError> {
        if account.owner != *program_id {
            return Err(VaultChainError::Owner {
                expected: *program_id,
                actual: account.owner,
            });
        }
        Self::parse(&account.data)
    }

    pub fn parse(data: &[u8]) -> Result<Self, VaultChainError> {
        if data.len() != Self::LEN {
            return Err(VaultChainError::AccountLength {
                expected: Self::LEN,
                actual: data.len(),
            });
        }
        let data = header(
            data,
            Self::LEN,
            PRICE_ROUTE_DISCRIMINATOR,
            PRICE_ROUTE_VERSION,
        )?;
        let len = usize::from(u16::from_le_bytes([data[4], data[5]]));
        if len > MAX_PRICE_ROUTE_BYTES {
            return Err(VaultChainError::PriceRoute("bricks exceed 640 bytes"));
        }
        let mut reader = Reader {
            bytes: &data[BRICKS_OFFSET..BRICKS_OFFSET + len],
        };
        let mut bricks = Vec::new();
        while !reader.bytes.is_empty() {
            if bricks.len() == MAX_PRICE_BRICKS {
                return Err(VaultChainError::PriceRoute("more than 6 bricks"));
            }
            bricks.push(PriceBrick::decode(&mut reader)?);
        }
        if bricks.is_empty() {
            return Err(VaultChainError::PriceRoute("route has no bricks"));
        }
        let accounts: usize = bricks.iter().map(PriceBrick::account_count).sum();
        if accounts != usize::from(data[3]) {
            return Err(VaultChainError::PriceRoute(
                "account count does not match the bricks",
            ));
        }
        Ok(Self {
            mint: pubkey(data, 8),
            max_skew_secs: u32::from_le_bytes([data[40], data[41], data[42], data[43]]),
            bricks,
            updated_slot: u64::from_le_bytes([
                data[688], data[689], data[690], data[691], data[692], data[693], data[694],
                data[695],
            ]),
        })
    }

    /// One leg of a price section: `address`, then its bricks' accounts, Pyth read on `shard`.
    #[must_use]
    pub fn accounts(&self, address: Pubkey, shard: u16) -> Vec<AccountMeta> {
        let mut accounts = vec![AccountMeta::new_readonly(address, false)];
        for brick in &self.bricks {
            match brick {
                PriceBrick::Pyth(pyth) => accounts.push(AccountMeta::new_readonly(
                    pyth_price_feed_account(&pyth.feed_id, shard),
                    false,
                )),
                PriceBrick::AccountRatio(ratio) => accounts.extend(
                    [ratio.account, ratio.program_data]
                        .map(|key| AccountMeta::new_readonly(key, false)),
                ),
                PriceBrick::MaturityDiscount(_)
                | PriceBrick::ClampMax { .. }
                | PriceBrick::GrowthCap(_) => {}
            }
        }
        accounts
    }

    pub fn account_ratios(&self) -> impl Iterator<Item = &AccountRatioBrick> {
        self.bricks.iter().filter_map(|brick| match brick {
            PriceBrick::AccountRatio(ratio) => Some(ratio),
            _ => None,
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], VaultChainError> {
        let (head, tail) = self
            .bytes
            .split_at_checked(len)
            .ok_or(VaultChainError::PriceRoute("brick is truncated"))?;
        self.bytes = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], VaultChainError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn pubkey(&mut self) -> Result<Pubkey, VaultChainError> {
        self.array().map(Pubkey::new_from_array)
    }

    fn u8(&mut self) -> Result<u8, VaultChainError> {
        self.array::<1>().map(|[byte]| byte)
    }

    fn u16(&mut self) -> Result<u16, VaultChainError> {
        self.array().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32, VaultChainError> {
        self.array().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, VaultChainError> {
        self.array().map(u64::from_le_bytes)
    }

    fn u128(&mut self) -> Result<u128, VaultChainError> {
        self.array().map(u128::from_le_bytes)
    }

    fn field(&mut self) -> Result<RatioField, VaultChainError> {
        Ok(RatioField {
            at: self.u16()?,
            width: self.u8()?,
        })
    }

    fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, VaultChainError>,
    ) -> Result<Option<T>, VaultChainError> {
        match self.u8()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(VaultChainError::PriceRoute("option tag must be 0 or 1")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn pyth(feed: u8) -> Vec<u8> {
        let mut out = vec![0];
        out.extend([feed; 32]);
        out.extend(60u32.to_le_bytes());
        out.extend(500u32.to_le_bytes());
        out
    }

    fn ratio(account: u8, discriminator: &[u8]) -> Vec<u8> {
        let mut out = vec![1];
        for byte in [account, account + 1, account + 2] {
            out.extend([byte; 32]);
        }
        out.extend(7u64.to_le_bytes());
        out.push(u8::try_from(discriminator.len()).unwrap());
        out.extend(discriminator);
        out.extend(300u16.to_le_bytes());
        out.extend([1, 8, 0]);
        out.extend([2, 1, 8, 10, 1, 16]);
        out.push((-3i8).to_le_bytes()[0]);
        out.extend(1u64.to_le_bytes());
        out.extend(2u128.to_le_bytes());
        out.extend(3u128.to_le_bytes());
        out.extend([1, 0]);
        out.extend(274u16.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out
    }

    fn discount() -> Vec<u8> {
        let mut out = vec![2];
        out.extend(1_800_000_000u64.to_le_bytes());
        for apy in [50_000u32, 71_246, 100_000] {
            out.extend(apy.to_le_bytes());
        }
        out.extend([1, 0]);
        out.extend(264u16.to_le_bytes());
        out.extend(268u16.to_le_bytes());
        out
    }

    fn route_bytes(bricks: &[Vec<u8>], account_count: u8) -> Vec<u8> {
        let blob = bricks.concat();
        let mut data = vec![0; PriceRoute::LEN];
        data[..4].copy_from_slice(&[15, 1, 255, account_count]);
        data[4..6].copy_from_slice(&u16::try_from(blob.len()).unwrap().to_le_bytes());
        data[8..40].fill(9);
        data[40..44].copy_from_slice(&30u32.to_le_bytes());
        data[BRICKS_OFFSET..BRICKS_OFFSET + blob.len()].copy_from_slice(&blob);
        data
    }

    fn pt_route() -> Vec<u8> {
        let mut clamp = vec![3];
        clamp.extend(1_000_000_000_000_000_000u128.to_le_bytes());
        let mut cap = vec![4];
        cap.extend(5u128.to_le_bytes());
        cap.extend(6u64.to_le_bytes());
        cap.extend(7u32.to_le_bytes());
        cap.extend(8u32.to_le_bytes());
        route_bytes(
            &[
                ratio(48, &[1, 2, 3, 4, 5, 6, 7, 8]),
                ratio(64, &[]),
                clamp,
                discount(),
                cap,
                pyth(16),
            ],
            5,
        )
    }

    #[test]
    fn price_feed_address_is_shard_and_feed_specific() {
        let first = [1; 32];
        let second = [2; 32];
        assert_ne!(
            pyth_price_feed_account(&first, 0),
            pyth_price_feed_account(&first, 1)
        );
        assert_ne!(
            pyth_price_feed_account(&first, 0),
            pyth_price_feed_account(&second, 0)
        );
    }

    #[test]
    fn sol_usd_shard_zero_matches_the_sponsored_feed_account() {
        let mut feed = [0; 32];
        hex::decode_to_slice(
            "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
            &mut feed,
        )
        .unwrap();
        assert_eq!(
            pyth_price_feed_account(&feed, 0).to_string(),
            "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE"
        );
    }

    #[test]
    fn route_decodes_bricks_and_lists_their_accounts_in_key_order() {
        let mut data = pt_route();
        data[688..696].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        assert!(matches!(
            PriceRoute::parse(&data[..688]),
            Err(VaultChainError::AccountLength {
                expected: 696,
                actual: 688
            })
        ));
        let route = PriceRoute::parse(&data).unwrap();
        assert_eq!(route.updated_slot, 0x0102_0304_0506_0708);
        assert_eq!(route.mint, key(9));
        assert_eq!(route.max_skew_secs, 30);
        assert_eq!(route.bricks.len(), 6);
        assert_eq!(
            route.bricks[0],
            PriceBrick::AccountRatio(AccountRatioBrick {
                account: key(48),
                owner: key(49),
                program_data: key(50),
                deploy_slot: 7,
                discriminator: vec![1, 2, 3, 4, 5, 6, 7, 8],
                min_len: 300,
                mint_at: Some(8),
                numerator: RatioField { at: 258, width: 8 },
                denominator: RatioField { at: 266, width: 16 },
                scale_exp: -3,
                min_denominator: 1,
                min: 2,
                max: 3,
                freshness: Some(RatioFreshness {
                    unit: FreshnessUnit::Epoch,
                    at: 274,
                    max_lag: 1,
                }),
            })
        );
        assert_eq!(
            route.bricks[2],
            PriceBrick::ClampMax {
                max: 1_000_000_000_000_000_000
            }
        );
        assert_eq!(
            route.bricks[3],
            PriceBrick::MaturityDiscount(MaturityDiscountBrick {
                maturity: 1_800_000_000,
                apy_low_ppm: 50_000,
                apy_mid_ppm: 71_246,
                apy_high_ppm: 100_000,
                maturity_source: Some(MaturitySource {
                    brick: 0,
                    start_at: 264,
                    duration_at: 268,
                }),
            })
        );
        assert_eq!(
            route.bricks[4],
            PriceBrick::GrowthCap(GrowthCapBrick {
                anchor_rate: 5,
                anchor_ts: 6,
                max_growth_ppm: 7,
                max_anchor_age_secs: 8,
            })
        );
        assert_eq!(
            route.bricks[5],
            PriceBrick::Pyth(PythBrick {
                feed_id: [16; 32],
                max_age_secs: 60,
                min_band_ppm: 500,
            })
        );
        assert_eq!(
            route
                .account_ratios()
                .map(|r| r.account)
                .collect::<Vec<_>>(),
            [key(48), key(64)]
        );
        let address = key(1);
        let accounts: Vec<_> = route
            .accounts(address, 3)
            .into_iter()
            .map(|meta| {
                assert!(!meta.is_signer && !meta.is_writable);
                meta.pubkey
            })
            .collect();
        assert_eq!(
            accounts,
            [
                address,
                key(48),
                key(50),
                key(64),
                key(66),
                pyth_price_feed_account(&[16; 32], 3),
            ]
        );
    }

    #[test]
    fn route_rejects_bad_shape() {
        assert!(PriceRoute::parse(&pt_route()).is_ok());
        assert!(PriceRoute::parse(&route_bytes(&[], 0)).is_err());
        assert!(PriceRoute::parse(&route_bytes(&[pyth(1)], 2)).is_err());
        assert!(PriceRoute::parse(&route_bytes(&vec![pyth(1); 7], 7)).is_err());
        assert!(PriceRoute::parse(&route_bytes(&[vec![5]], 0)).is_err());
        let mut truncated = route_bytes(&[pyth(1)], 1);
        truncated[4] = 40;
        assert!(PriceRoute::parse(&truncated).is_err());
        let mut long = route_bytes(&[pyth(1)], 1);
        long[4..6].copy_from_slice(&641u16.to_le_bytes());
        assert!(PriceRoute::parse(&long).is_err());
        let mut tag = ratio(48, &[]);
        let at = tag.len() - 8;
        tag[at] = 2;
        assert!(PriceRoute::parse(&route_bytes(&[tag, pyth(1)], 3)).is_err());
        assert!(PriceRoute::parse(&route_bytes(&[ratio(48, &[0; 9]), pyth(1)], 3)).is_err());
        let mut version = route_bytes(&[pyth(1)], 1);
        version[1] = 2;
        assert_eq!(
            PriceRoute::parse(&version),
            Err(VaultChainError::Version(2))
        );
        let mut discriminator = route_bytes(&[pyth(1)], 1);
        discriminator[0] = 7;
        assert!(PriceRoute::parse(&discriminator).is_err());
        assert!(PriceRoute::parse(&route_bytes(&[pyth(1)], 1)[..687]).is_err());
    }
}
