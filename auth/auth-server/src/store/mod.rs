use std::collections::{HashMap, VecDeque};

use auth_server_api::GasSponsorshipInfo;
use renegade_circuit_types::wallet::Nullifier;
use tokio::sync::Mutex;

use crate::error::AuthServerError;

pub mod helpers;

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
    /// bundle_id → context
    by_id: HashMap<String, BundleContext>,
    /// nullifier → queue of bundle_ids
    /// TODO: Maybe use String instead of Nullifier
    /// TODO: Is VecDeque the best data structure here?
    by_null: HashMap<Nullifier, VecDeque<String>>,
}

pub struct BundleStore {
    inner: Mutex<StoreInner>,
}

impl BundleStore {
    pub fn new() -> Self {
        Self { inner: Mutex::new(StoreInner { by_id: HashMap::new(), by_null: HashMap::new() }) }
    }
}

impl BundleStore {
    /// We use `lock().await` so writers queue up on the Mutex
    /// *behind* any in‐flight writer, but *ahead* of any try_lock readers.
    /// This guarantees that writes never block behind cleanup or read.
    pub async fn write(
        &self,
        bundle_id: String,
        ctx: BundleContext,
    ) -> Result<(), AuthServerError> {
        let mut inner = self.inner.lock().await;
        inner.by_id.insert(bundle_id.clone(), ctx.clone());
        inner.by_null.entry(ctx.nullifier).or_insert_with(VecDeque::new).push_back(bundle_id);
        Ok(())
    }

    /// We use `try_lock()` + `yield_now()` instead of `lock().await`
    /// so that if a writer is pending or holding the lock, we
    /// never suspend the writer.  `try_lock()` fails immediately,
    /// and we `yield_now()` to give the executor a chance to schedule
    /// the writer next.  Once the lock is free, we grab it instantly.
    pub async fn read(&self, bundle_id: &str) -> Result<Option<BundleContext>, AuthServerError> {
        loop {
            if let Ok(idx) = self.inner.try_lock() {
                return Ok(idx.by_id.get(bundle_id).cloned());
            }
            tokio::task::yield_now().await;
        }
    }

    /// Remove a nullifier and all associated bundle ids from the store
    ///
    /// Because each nullifier queue is ≤20 entries, we can safely
    /// do a single `lock().await` and perform O(1 + k) pops/deletes
    /// without noticeable writer blocking.  If this ever grows,
    /// we'd switch to try_lock+yield similar to `read`.
    pub async fn _cleanup_by_nullifier(
        &self,
        nullifier: &Nullifier,
    ) -> Result<(), AuthServerError> {
        let mut inner = self.inner.lock().await;
        let bundle_ids = inner.by_null.remove(nullifier).unwrap_or_default();
        for bundle_id in bundle_ids {
            inner.by_id.remove(&bundle_id);
        }
        Ok(())
    }
}
