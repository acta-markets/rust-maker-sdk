use rand::rngs::OsRng;
use rand::{RngCore, SeedableRng, TryRngCore};
use rand_xoshiro::Xoshiro256PlusPlus;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct NonceError(String);

impl std::fmt::Display for NonceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NonceError {}

pub struct NonceGenerator {
    rng: Xoshiro256PlusPlus,
}

impl NonceGenerator {
    pub fn new() -> Result<Self, NonceError> {
        let mut seed = [0u8; 32];
        let mut rng = OsRng;
        rng.try_fill_bytes(&mut seed)
            .map_err(|e| NonceError(format!("OsRng unavailable: {e}")))?;
        Ok(Self {
            rng: Xoshiro256PlusPlus::from_seed(seed),
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }
}

/// Lock-free atomic nonce generator for concurrent use.
///
/// Uses atomic counter XOR'd with entropy base for uniqueness.
/// Call [`init()`](Self::init) once before use to seed from OS entropy.
pub struct AtomicNonceGenerator {
    counter: AtomicU64,
    entropy_base: AtomicU64,
}

impl AtomicNonceGenerator {
    pub const fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
            entropy_base: AtomicU64::new(0),
        }
    }

    pub fn init(&self) -> Result<(), NonceError> {
        if self.entropy_base.load(Ordering::Relaxed) == 0 {
            let mut buf = [0u8; 8];
            let mut rng = OsRng;
            rng.try_fill_bytes(&mut buf)
                .map_err(|e| NonceError(format!("OsRng unavailable: {e}")))?;
            let entropy = u64::from_le_bytes(buf);
            let _ =
                self.entropy_base
                    .compare_exchange(0, entropy, Ordering::SeqCst, Ordering::Relaxed);
        }
        Ok(())
    }

    #[inline]
    pub fn next_u64(&self) -> u64 {
        let base = self.entropy_base.load(Ordering::Relaxed);
        let base = if base == 0 {
            if let Err(e) = self.init() {
                tracing::warn!(
                    "AtomicNonceGenerator: OsRng unavailable ({e}), nonces will be sequential"
                );
            }
            self.entropy_base.load(Ordering::Relaxed)
        } else {
            base
        };

        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        base.wrapping_add(count) ^ base.rotate_left((count & 63) as u32)
    }
}

impl Default for AtomicNonceGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "nonce_tests.rs"]
mod tests;
