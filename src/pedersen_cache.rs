use std::{
    cell::RefCell,
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use starknet_types_core::felt::Felt;

pub const DEFAULT_PEDERSEN_CACHE_CAPACITY: usize = 8 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(false);
static CAPACITY: AtomicUsize = AtomicUsize::new(DEFAULT_PEDERSEN_CACHE_CAPACITY);
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static CAPACITY_CLEARS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static CACHE: RefCell<HashMap<(Felt, Felt), Felt>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PedersenCacheConfig {
    pub enabled: bool,
    /// Maximum entries retained by each native execution thread.
    pub capacity: usize,
}

impl Default for PedersenCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            capacity: DEFAULT_PEDERSEN_CACHE_CAPACITY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PedersenCacheMetrics {
    pub total_calls: u64,
    pub hits: u64,
    pub misses: u64,
    pub capacity_clears: u64,
    /// Configured capacity for each native execution thread.
    pub capacity: usize,
}

/// Configures native-runtime Pedersen memoization.
///
/// Configure this once during process startup, before native execution workers
/// begin handling transactions.
pub fn configure_pedersen_cache(config: PedersenCacheConfig) {
    ENABLED.store(false, Ordering::Relaxed);
    CAPACITY.store(config.capacity, Ordering::Relaxed);
    ENABLED.store(config.enabled, Ordering::Relaxed);
}

/// Enables or disables native-runtime Pedersen memoization.
///
/// Configure this once during process startup, before native execution workers
/// begin handling transactions.
pub fn set_pedersen_cache_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn pedersen_cache_metrics() -> PedersenCacheMetrics {
    let hits = HITS.load(Ordering::Relaxed);
    let misses = MISSES.load(Ordering::Relaxed);
    PedersenCacheMetrics {
        total_calls: hits.saturating_add(misses),
        hits,
        misses,
        capacity_clears: CAPACITY_CLEARS.load(Ordering::Relaxed),
        capacity: CAPACITY.load(Ordering::Relaxed),
    }
}

pub(crate) fn get(lhs: Felt, rhs: Felt) -> Option<Felt> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    match CACHE.with(|cache| cache.borrow().get(&(lhs, rhs)).copied()) {
        Some(result) => {
            HITS.fetch_add(1, Ordering::Relaxed);
            Some(result)
        }
        None => {
            MISSES.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

pub(crate) fn insert(lhs: Felt, rhs: Felt, result: Felt) {
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
        cache.insert((lhs, rhs), result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear() {
        CACHE.with(|cache| cache.borrow_mut().clear());
    }

    #[test]
    fn cache_is_opt_in() {
        let _guard = TEST_MUTEX.lock().unwrap();
        let lhs = Felt::from(1);
        let rhs = Felt::from(3);
        let result = Felt::from(5);

        configure_pedersen_cache(PedersenCacheConfig::default());
        clear();
        insert(lhs, rhs, result);
        assert_eq!(get(lhs, rhs), None);

        set_pedersen_cache_enabled(true);
        insert(lhs, rhs, result);
        assert_eq!(get(lhs, rhs), Some(result));

        set_pedersen_cache_enabled(false);
        clear();
    }

    #[test]
    fn capacity_and_metrics_are_configurable() {
        let _guard = TEST_MUTEX.lock().unwrap();
        configure_pedersen_cache(PedersenCacheConfig {
            enabled: true,
            capacity: 1,
        });
        clear();
        let before = pedersen_cache_metrics();

        let first = Felt::from(1);
        let second = Felt::from(2);
        let result = Felt::from(3);
        assert_eq!(get(first, first), None);
        insert(first, first, result);
        assert_eq!(get(first, first), Some(result));
        insert(second, second, result);

        let after = pedersen_cache_metrics();
        assert_eq!(after.capacity, 1);
        assert!(after.total_calls >= before.total_calls + 2);
        assert!(after.hits > before.hits);
        assert!(after.misses > before.misses);
        assert!(after.capacity_clears > before.capacity_clears);
        CACHE.with(|cache| assert_eq!(cache.borrow().len(), 1));

        configure_pedersen_cache(PedersenCacheConfig::default());
        clear();
    }
}
