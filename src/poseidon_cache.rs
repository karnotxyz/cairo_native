use std::{
    cell::RefCell,
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use starknet_types_core::felt::Felt;

pub const DEFAULT_POSEIDON_CACHE_CAPACITY: usize = 8 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(false);
static CAPACITY: AtomicUsize = AtomicUsize::new(DEFAULT_POSEIDON_CACHE_CAPACITY);
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static CAPACITY_CLEARS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static CACHE: RefCell<HashMap<[Felt; 3], [Felt; 3]>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoseidonCacheConfig {
    pub enabled: bool,
    pub capacity: usize,
}

impl Default for PoseidonCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            capacity: DEFAULT_POSEIDON_CACHE_CAPACITY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoseidonCacheMetrics {
    pub total_calls: u64,
    pub hits: u64,
    pub misses: u64,
    pub capacity_clears: u64,
    pub capacity: usize,
}

/// Configure once at startup, before native execution workers begin.
pub fn configure_poseidon_cache(config: PoseidonCacheConfig) {
    CAPACITY.store(config.capacity, Ordering::Relaxed);
    ENABLED.store(config.enabled, Ordering::Relaxed);
}

pub fn poseidon_cache_metrics() -> PoseidonCacheMetrics {
    let hits = HITS.load(Ordering::Relaxed);
    let misses = MISSES.load(Ordering::Relaxed);
    PoseidonCacheMetrics {
        total_calls: hits.saturating_add(misses),
        hits,
        misses,
        capacity_clears: CAPACITY_CLEARS.load(Ordering::Relaxed),
        capacity: CAPACITY.load(Ordering::Relaxed),
    }
}

pub(crate) fn get(input: [Felt; 3]) -> Option<[Felt; 3]> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    match CACHE.with(|cache| cache.borrow().get(&input).copied()) {
        Some(output) => {
            HITS.fetch_add(1, Ordering::Relaxed);
            Some(output)
        }
        None => {
            MISSES.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

pub(crate) fn insert(input: [Felt; 3], output: [Felt; 3]) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let capacity = CAPACITY.load(Ordering::Relaxed);
        if capacity == 0 {
            return;
        }
        if cache.len() >= capacity {
            cache.clear();
            CAPACITY_CLEARS.fetch_add(1, Ordering::Relaxed);
        }
        cache.insert(input, output);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn opt_in_and_bounded() {
        let _guard = TEST_MUTEX.lock().unwrap();
        let input = [Felt::from(1), Felt::from(2), Felt::from(3)];
        let output = [Felt::from(4), Felt::from(5), Felt::from(6)];
        configure_poseidon_cache(PoseidonCacheConfig::default());
        CACHE.with(|cache| cache.borrow_mut().clear());
        insert(input, output);
        assert_eq!(get(input), None);

        configure_poseidon_cache(PoseidonCacheConfig {
            enabled: true,
            capacity: 1,
        });
        insert(input, output);
        assert_eq!(get(input), Some(output));
        insert([Felt::from(7), Felt::from(8), Felt::from(9)], output);
        assert_eq!(get(input), None);
        assert!(poseidon_cache_metrics().capacity_clears > 0);

        configure_poseidon_cache(PoseidonCacheConfig::default());
        CACHE.with(|cache| cache.borrow_mut().clear());
    }
}
