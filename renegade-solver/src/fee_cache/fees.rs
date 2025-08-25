//! Defines a cache for the base fee per gas.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The default value for the base fee per gas.
const NONE: u64 = 0;

/// The inner cache backing storage. Kept private to hide concurrency details.
struct FeeCacheInner {
    /// The base fee per gas.
    base_fee_per_gas: AtomicU64,
}

/// A value-type handle to the fee cache. Clones are cheap and share state.
#[derive(Clone)]
pub struct FeeCache(Arc<FeeCacheInner>);

impl Default for FeeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FeeCache {
    /// Create a new cache
    pub fn new() -> Self {
        Self(Arc::new(FeeCacheInner { base_fee_per_gas: AtomicU64::new(NONE) }))
    }

    /// Sets the base fee per gas.
    pub fn set_base_fee_per_gas(&self, base: u64) {
        self.0.base_fee_per_gas.store(base, Ordering::Relaxed);
    }

    /// Gets the base fee per gas.
    pub fn base_fee_per_gas(&self) -> Option<u64> {
        match self.0.base_fee_per_gas.load(Ordering::Relaxed) {
            NONE => None,
            v => Some(v),
        }
    }
}
