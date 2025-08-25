//! Defines a cache for the base fee per gas.
use std::sync::atomic::{AtomicU64, Ordering};

/// The default value for the base fee per gas.
const NONE: u64 = 0;

/// The cache for the base fee per gas.
pub struct FeeCache {
    /// The base fee per gas.
    base_fee_per_gas: AtomicU64,
}

impl Default for FeeCache {
    fn default() -> Self {
        Self { base_fee_per_gas: AtomicU64::new(NONE) }
    }
}

impl FeeCache {
    /// Sets the base fee per gas.
    pub fn set_base_fee_per_gas(&self, base: u64) {
        self.base_fee_per_gas.store(base, Ordering::Relaxed);
    }

    /// Gets the base fee per gas.
    pub fn base_fee_per_gas(&self) -> Option<u64> {
        match self.base_fee_per_gas.load(Ordering::Relaxed) {
            NONE => None,
            v => Some(v),
        }
    }
}
