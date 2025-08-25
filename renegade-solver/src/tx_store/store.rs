//! Triggered transaction store for L2 transactions.
//!
//! Tracks queued transactions keyed by ID, the L2 trigger at which they should
//! be sent, and their inclusion status. Resolves fee caps on demand using a
//! `FeeCache`. Hash-based queries are performed by linear scan (no index).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use alloy::rpc::types::TransactionRequest;
use alloy_primitives::B256;

use crate::fee_cache::FeeCache;
use crate::tx_store::error::{TxStoreError, TxStoreResult};

/// A position on the L2 chain (block and flashblock).
#[derive(Clone, Debug)]
pub struct L2Position {
    /// The L2 block number.
    pub l2_block: u64,
    /// The flashblock number.
    pub flashblock: u64,
}

impl L2Position {
    /// Returns true if this position equals the given coordinates.
    pub fn equals(&self, l2_block: u64, flashblock: u64) -> bool {
        self.l2_block == l2_block && self.flashblock == flashblock
    }
}

/// Timing information for when a transaction becomes eligible to send.
#[derive(Clone, Debug)]
pub struct TxTiming {
    /// The trigger position at which the transaction becomes eligible to send.
    pub trigger: L2Position,
    /// A buffer time in milliseconds before sending after the trigger is seen.
    pub buffer_ms: u64,
}

impl TxTiming {
    /// Returns true if the provided position matches this timing's trigger.
    pub fn triggers_at(&self, l2_block: u64, flashblock: u64) -> bool {
        self.trigger.equals(l2_block, flashblock)
    }
}

/// Status information captured as the transaction progresses through the chain.
#[derive(Clone, Debug, Default)]
pub struct TxStatus {
    /// The hash of the broadcast transaction (if known).
    pub tx_hash: Option<B256>,
    /// The observed inclusion position (if seen).
    pub observed: Option<L2Position>,
}

/// A queued transaction with trigger timing and mutable status.
#[derive(Clone, Debug)]
pub struct TxContext {
    /// The ID of the transaction.
    pub id: String,
    /// The template request for the transaction (no nonce/max_fee_per_gas set).
    pub request: TransactionRequest,
    /// The timing information for when to send.
    pub timing: TxTiming,
    /// The evolving status of this transaction.
    pub status: TxStatus,
}

#[derive(Default)]
struct StoreInner {
    /// Transactions by ID.
    by_id: HashMap<String, TxContext>,
}

/// A thread-safe store for transactions that are sent on specific L2 triggers.
#[derive(Clone)]
pub struct TxStore {
    /// The inner store.
    inner: Arc<RwLock<StoreInner>>,
    /// Fee source to compute max_fee_per_gas at send time.
    fee_cache: FeeCache,
}

impl TxStore {
    /// Creates a new `TxStore` with the given fee cache.
    pub fn new(fee_cache: FeeCache) -> Self {
        Self { inner: Arc::new(RwLock::new(StoreInner::default())), fee_cache }
    }

    // --------------
    // | Public API |
    // --------------

    /// Enqueues a transaction with the given timing.
    pub fn enqueue_with_timing(
        &self,
        id: &str,
        request: TransactionRequest,
        timing: TxTiming,
    ) -> TxStoreResult<()> {
        let tx = TxContext { id: id.to_string(), request, timing, status: TxStatus::default() };
        let mut inner = self.inner.write().unwrap();

        inner.by_id.insert(tx.id.clone(), tx);
        Ok(())
    }

    /// Returns the transactions that are due to send at the specified trigger
    /// position, along with the time each should be sent (after buffering).
    pub fn due_at(
        &self,
        l2_block: u64,
        flashblock: u64,
        received_at: Instant,
    ) -> Vec<(String, Instant)> {
        let inner = self.inner.read().unwrap();
        inner
            .by_id
            .iter()
            .filter_map(|(id, tx)| {
                if tx.timing.triggers_at(l2_block, flashblock) {
                    let at = received_at
                        .checked_add(std::time::Duration::from_millis(tx.timing.buffer_ms))
                        .unwrap_or(received_at);
                    Some((id.clone(), at))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Resolves the transaction template into a concrete request with fee caps.
    pub fn resolve_fee_caps(&self, id: &str) -> TxStoreResult<TransactionRequest> {
        let tx = {
            let inner = self.inner.read().unwrap();
            inner
                .by_id
                .get(id)
                .cloned()
                .ok_or_else(|| TxStoreError::TxNotFound { id: id.to_string() })?
        };

        // Read latest base fee from cache.
        let base = self.fee_cache.base_fee_per_gas().ok_or_else(|| {
            TxStoreError::TxRequestInvalid("base_fee_per_gas unavailable".to_string())
        })? as u128;
        // 1.2 * base
        let buffed_base_fee = base.saturating_mul(12) / 10;
        // 1.2 * base + tip

        let tip = tx.request.max_priority_fee_per_gas.ok_or_else(|| {
            TxStoreError::TxRequestInvalid("max_priority_fee_per_gas must be set".to_string())
        })?;

        let max_fee = buffed_base_fee.saturating_add(tip);

        let mut out = tx.request.clone();
        out.max_fee_per_gas = Some(max_fee);

        Ok(out)
    }

    /// Attaches or updates the transaction hash for a queued transaction.
    pub fn record_tx_hash(&self, id: &str, tx_hash: B256) {
        let mut inner = self.inner.write().unwrap();
        if let Some(tx) = inner.by_id.get_mut(id) {
            tx.status.tx_hash = Some(tx_hash);
        }
    }

    /// Marks transactions as observed included at the given position if their
    /// hashes match the provided set. Returns the `(id, hash)` pairs that
    /// matched.
    pub fn record_inclusions(
        &self,
        position: &L2Position,
        tx_hashes: &HashSet<B256>,
    ) -> Vec<(String, B256)> {
        let mut inner = self.inner.write().unwrap();
        let mut out: Vec<(String, B256)> = Vec::new();
        for (id, tx) in inner.by_id.iter_mut() {
            if let Some(h) = tx.status.tx_hash {
                if tx_hashes.contains(&h) {
                    tx.status.observed = Some(position.clone());
                    out.push((id.clone(), h));
                }
            }
        }
        out
    }
}
