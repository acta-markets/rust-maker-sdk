//! Pure venue decoders and CPI hop builders.
pub mod exponent;
pub mod jupiter;

use super::{VaultChainError, VaultSwapHop};
use solana_sdk::pubkey::Pubkey;

pub(super) fn invalid(message: &'static str) -> VaultChainError {
    VaultChainError::InvalidInput(message)
}

#[derive(Debug, Clone)]
pub struct SwapVenues(Vec<(Pubkey, Vec<Vec<u8>>)>);

impl SwapVenues {
    pub fn parse(data: &[u8]) -> Result<Self, VaultChainError> {
        if data.len() != 3364 || data[0] != 16 || data[1] != 1 || data[3] > 32 {
            return Err(invalid("invalid SwapVenues account"));
        }
        let mut venues = Vec::new();
        for slot in data[4..]
            .as_chunks::<105>()
            .0
            .iter()
            .take(usize::from(data[3]))
        {
            let mut r = Reader::new(slot, 0);
            let program = r.key()?;
            let count = r.byte()?;
            if !(1..=8).contains(&count) {
                return Err(invalid("invalid venue selector count"));
            }
            let mut selectors = Vec::new();
            for _ in 0..count {
                let len = usize::from(r.byte()?);
                let bytes = r.take(8)?;
                if !(1..=8).contains(&len) || bytes[len..].iter().any(|b| *b != 0) {
                    return Err(invalid("invalid venue selector"));
                }
                selectors.push(bytes[..len].to_vec());
            }
            venues.push((program, selectors));
        }
        Ok(Self(venues))
    }

    pub fn allows(&self, hop: &VaultSwapHop) -> bool {
        self.0.iter().any(|(program, selectors)| {
            *program == hop.program
                && selectors.iter().any(|s| {
                    hop.data.starts_with(s)
                        && hop
                            .amount_offset
                            .is_none_or(|offset| s.len() <= usize::from(offset.offset))
                })
        })
    }
}

pub(super) struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    pub(super) fn new(data: &'a [u8], at: usize) -> Self {
        Self { data, at }
    }
    pub(super) fn take(&mut self, len: usize) -> Result<&'a [u8], VaultChainError> {
        let end = self
            .at
            .checked_add(len)
            .ok_or(invalid("venue offset overflow"))?;
        let bytes = self
            .data
            .get(self.at..end)
            .ok_or(invalid("truncated venue account"))?;
        self.at = end;
        Ok(bytes)
    }
    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], VaultChainError> {
        self.take(N)?
            .try_into()
            .map_err(|_| invalid("truncated venue field"))
    }
    pub(super) fn byte(&mut self) -> Result<u8, VaultChainError> {
        Ok(self.take(1)?[0])
    }
    pub(super) fn u32(&mut self) -> Result<u32, VaultChainError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub(super) fn u64(&mut self) -> Result<u64, VaultChainError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub(super) fn key(&mut self) -> Result<Pubkey, VaultChainError> {
        Ok(Pubkey::new_from_array(self.array()?))
    }
    pub(super) fn skip_vec(&mut self, width: usize) -> Result<(), VaultChainError> {
        let count = self.u32()? as usize;
        self.take(
            count
                .checked_mul(width)
                .ok_or(invalid("venue vector overflow"))?,
        )?;
        Ok(())
    }
}
