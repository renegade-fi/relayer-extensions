//! Defines a cache for the base fee per gas.
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::Address;

use crate::cli::Cli;

/// The inner cache backing storage.
struct ChainStateCacheInner {
    /// The base fee per gas.
    base_fee_per_gas: AtomicU64,
    /// The pending nonce for the signer address.
    pending_nonce: AtomicU64,
    /// Signer whose pending nonce is tracked
    signer_address: Address,
}

#[derive(Clone)]
/// A value-type handle to the chain state cache. Clones are cheap and share
/// state.
pub struct ChainStateCache(Arc<ChainStateCacheInner>);

impl ChainStateCache {
    /// Create a new cache
    pub fn new(cli: &Cli) -> Self {
        let signer = PrivateKeySigner::from_str(&cli.private_key).expect("Failed to parse signer");
        let signer_address = signer.address();
        Self(Arc::new(ChainStateCacheInner {
            base_fee_per_gas: AtomicU64::default(),
            pending_nonce: AtomicU64::default(),
            signer_address,
        }))
    }

    /// Sets the base fee per gas.
    pub fn set_base_fee_per_gas(&self, base: u64) {
        self.0.base_fee_per_gas.store(base, Ordering::Relaxed);
    }

    /// Gets the base fee per gas.
    pub fn base_fee_per_gas(&self) -> Option<u64> {
        match self.0.base_fee_per_gas.load(Ordering::Relaxed) {
            0 => None,
            v => Some(v),
        }
    }

    /// Sets the pending nonce for the signer address.
    pub fn set_pending_nonce(&self, nonce: u64) {
        self.0.pending_nonce.store(nonce, Ordering::Relaxed);
    }

    /// Gets the pending nonce for the signer address.
    pub fn pending_nonce(&self) -> Option<u64> {
        match self.0.pending_nonce.load(Ordering::Relaxed) {
            0 => None,
            v => Some(v),
        }
    }

    /// Gets the signer address.
    pub fn signer_address(&self) -> Address {
        self.0.signer_address
    }
}
