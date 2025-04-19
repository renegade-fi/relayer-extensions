//! Defines the bundle store and associated types
use std::{collections::HashMap, sync::Arc};

use auth_server_api::GasSponsorshipInfo;
use renegade_circuit_types::wallet::Nullifier;
use tokio::sync::RwLock;

use crate::error::AuthServerError;

pub mod helpers;

/// Context of an external match bundle
#[derive(Clone)]
pub struct BundleContext {
    /// The key description that settled the bundle
    pub key_description: String,
    /// The request ID of the bundle
    pub request_id: String,
    /// The SDK version that requested the bundle
    pub sdk_version: String,
    /// The gas sponsorship info for the bundle
    pub gas_sponsorship_info: Option<GasSponsorshipInfo>,
    /// Whether the bundle was sponsored
    pub is_sponsored: bool,
    /// The nullifier that was nullified as a result of the bundle being settled
    pub nullifier: Nullifier,
}

struct StoreInner {
    by_id: HashMap<String, BundleContext>,
    by_null: HashMap<Nullifier, Vec<String>>,
}

impl StoreInner {
    pub fn new() -> Self {
        Self { by_id: HashMap::new(), by_null: HashMap::new() }
    }
}

/// A thread-safe store for tracking bundle contexts by ID and nullifier.
#[derive(Clone)]
pub struct BundleStore {
    inner: Arc<RwLock<StoreInner>>,
}

impl BundleStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(StoreInner::new())) }
    }

    pub async fn write(
        &self,
        bundle_id: String,
        ctx: BundleContext,
    ) -> Result<(), AuthServerError> {
        let mut inner = self.inner.write().await;
        inner.by_id.insert(bundle_id.clone(), ctx.clone());
        inner.by_null.entry(ctx.nullifier).or_default().push(bundle_id);
        Ok(())
    }

    pub async fn read(&self, bundle_id: &str) -> Result<Option<BundleContext>, AuthServerError> {
        let inner = self.inner.read().await;
        Ok(inner.by_id.get(bundle_id).cloned())
    }

    pub async fn _cleanup_by_nullifier(
        &self,
        nullifier: &Nullifier,
    ) -> Result<(), AuthServerError> {
        let mut inner = self.inner.write().await;
        if let Some(bundle_ids) = inner.by_null.remove(nullifier) {
            for bundle_id in bundle_ids {
                inner.by_id.remove(&bundle_id);
            }
        }
        Ok(())
    }
}
