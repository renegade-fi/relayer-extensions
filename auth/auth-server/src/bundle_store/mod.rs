//! Defines the bundle store and associated types

use std::sync::Arc;

use alloy_primitives::U256;
use auth_server_api::GasSponsorshipInfo;
use dashmap::DashMap;

/// The bundle ID type
pub type BundleId = U256;

// ------------------
// | Bundle Context |
// ------------------

/// Context of an external match bundle
#[derive(Clone, Debug)]
pub(crate) struct BundleContext {
    /// The key description that settled the bundle
    pub key_description: String,
    /// The bundle ID
    pub bundle_id: BundleId,
    /// The request ID of the bundle
    pub request_id: String,
    /// The SDK version that requested the bundle
    pub sdk_version: String,
    /// The gas sponsorship info for the bundle, along with the sponsorship
    /// nonce
    pub gas_sponsorship_info: Option<(GasSponsorshipInfo, U256)>,
    /// Whether the bundle was sponsored
    pub is_sponsored: bool,
    /// The timestamp of the price of the match in milliseconds
    pub price_timestamp: u64,
    /// The timestamp of the assembly of the bundle in milliseconds
    pub assembled_timestamp: Option<u64>,
}

// ---------
// | Store |
// ---------

/// A thread-safe store for tracking bundle contexts by ID and nullifier.
#[derive(Clone)]
pub struct BundleStore {
    /// The mapping from bundle ID to bundle context
    by_id: Arc<DashMap<BundleId, BundleContext>>,
}

impl BundleStore {
    /// Create a new bundle store
    pub fn new() -> Self {
        Self { by_id: Arc::new(DashMap::new()) }
    }

    /// Write a bundle to the store
    pub fn write(&self, ctx: BundleContext) {
        self.by_id.insert(ctx.bundle_id, ctx);
    }

    /// Read a bundle from the store by its ID
    pub fn read(&self, bundle_id: &BundleId) -> Option<BundleContext> {
        self.by_id.get(bundle_id).map(|ptr| ptr.value().clone())
    }

    /// Cleanup (remove) the bundle with the given ID
    pub fn remove_bundle(&self, bundle_id: &BundleId) {
        self.by_id.remove(bundle_id);
    }
}
