use super::*;
use std::collections::HashSet;
use std::thread;

#[test]
fn nonce_generator_produces_unique_values() {
    let mut nonce_gen = NonceGenerator::new().unwrap();
    let mut seen = HashSet::new();
    for _ in 0..10000 {
        assert!(seen.insert(nonce_gen.next_u64()));
    }
}

#[test]
fn atomic_nonce_generator_produces_unique_values() {
    let nonce_gen = AtomicNonceGenerator::new();
    nonce_gen.init().unwrap();
    let mut seen = HashSet::new();
    for _ in 0..10000 {
        assert!(seen.insert(nonce_gen.next_u64()));
    }
}

#[test]
fn atomic_nonce_generator_thread_safe() {
    static GEN: AtomicNonceGenerator = AtomicNonceGenerator::new();
    GEN.init().unwrap();

    let results: Vec<_> = (0..8)
        .map(|_| thread::spawn(|| (0..1000).map(|_| GEN.next_u64()).collect::<Vec<_>>()))
        .collect();

    let mut all_nonces = HashSet::new();
    for handle in results {
        for nonce in handle.join().unwrap() {
            assert!(all_nonces.insert(nonce), "duplicate nonce detected");
        }
    }
    assert_eq!(all_nonces.len(), 8000);
}

#[test]
fn atomic_nonce_generator_first_value_is_not_zero() {
    let nonce_gen = AtomicNonceGenerator::new();
    nonce_gen.init().unwrap();
    let first = nonce_gen.next_u64();
    assert_ne!(first, 0, "first nonce must not be zero");
}
