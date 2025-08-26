//! Triggered transaction store for L2 transactions.
//!
//! Tracks queued transactions keyed by ID, the L2 trigger at which they should
//! be sent, and their inclusion status. Resolves fee caps on demand using a
//! `FeeCache`. Hash-based queries are performed by linear scan (no index).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use alloy::rpc::types::TransactionRequest;
use alloy_primitives::B256;

use crate::fee_cache::FeeCache;
use crate::tx_store::error::{TxStoreError, TxStoreResult};
use dashmap::DashMap;

type OrderHash = String;

/// A position on the L2 chain (block and flashblock).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct L2Position {
    /// The L2 block number.
    pub l2_block: u64,
    /// The flashblock number.
    pub flashblock: u64,
}

/// Timing information for when a transaction becomes eligible to send.
#[derive(Clone, Debug)]
pub struct TxTiming {
    /// The trigger position at which the transaction becomes eligible to send.
    pub trigger: L2Position,
    /// The extra buffer time to wait beyond the flashblock trigger, for
    /// intra-flashblock accuracy.
    pub buffer_ms: u64,
}

impl TxTiming {
    /// Returns true if the provided position matches this timing's trigger.
    pub fn triggers_at(&self, at: &L2Position) -> bool {
        &self.trigger == at
    }
}

/// Status information captured as the transaction progresses through the chain.
#[derive(Clone, Debug, Default)]
pub struct TxStatus {
    /// The hash of the broadcast transaction (if known).
    pub tx_hash: Option<B256>,
    /// The observed inclusion position (if seen).
    pub observed_position: Option<L2Position>,
}

/// A queued transaction with trigger timing and mutable status.
#[derive(Clone, Debug)]
pub struct TxContext {
    /// The ID of the transaction. In practice, this is the order hash of the
    /// UniswapX order.
    pub id: OrderHash,
    /// The template request for the transaction (no nonce/max_fee_per_gas set).
    pub request: TransactionRequest,
    /// The timing information for when to send.
    pub timing: TxTiming,
    /// The evolving status of this transaction.
    pub status: TxStatus,
}

/// A thread-safe store for transactions that are sent on specific L2 triggers.
#[derive(Clone)]
pub struct TxStore {
    /// The inner store.
    by_id: Arc<DashMap<OrderHash, TxContext>>,
    /// Fee source to compute max_fee_per_gas at send time.
    fee_cache: FeeCache,
}

impl TxStore {
    /// Creates a new `TxStore` with the given fee cache.
    pub fn new(fee_cache: FeeCache) -> Self {
        Self { by_id: Arc::new(DashMap::new()), fee_cache }
    }

    // --------------
    // | Public API |
    // --------------

    /// Enqueues a transaction with the given timing.
    pub fn enqueue_with_timing(
        &self,
        id: &OrderHash,
        request: TransactionRequest,
        timing: TxTiming,
    ) -> TxStoreResult<()> {
        let tx = TxContext { id: id.to_string(), request, timing, status: TxStatus::default() };
        self.by_id.insert(tx.id.clone(), tx);
        Ok(())
    }

    /// Returns the transactions that are due to send at the specified trigger
    /// position, along with the time each should be sent (after buffering).
    ///
    /// `received_at` is the timestamp when the trigger position was observed
    /// locally; it is used as the base to apply the per-transaction buffer.
    pub fn due_at(&self, at: &L2Position, received_at: Instant) -> Vec<(String, Instant)> {
        self.by_id
            .iter()
            .filter_map(|entry| {
                let id = entry.key();
                let tx = entry.value();
                if tx.timing.triggers_at(at) {
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
    pub fn resolve_fee_caps(&self, id: &OrderHash) -> TxStoreResult<TransactionRequest> {
        let tx_ref =
            self.by_id.get(id).ok_or_else(|| TxStoreError::TxNotFound { id: id.to_string() })?;

        // Read latest base fee from cache.
        let base = self.fee_cache.base_fee_per_gas().ok_or_else(|| {
            TxStoreError::TxRequestInvalid("base_fee_per_gas unavailable".to_string())
        })? as u128;

        let tip = tx_ref.request.max_priority_fee_per_gas.ok_or_else(|| {
            TxStoreError::TxRequestInvalid("max_priority_fee_per_gas must be set".to_string())
        })?;

        // Add 20% buffer to the base fee.
        let buffed_base_fee = base.saturating_mul(12) / 10;
        let max_fee = buffed_base_fee.saturating_add(tip);

        let mut out = tx_ref.request.clone();
        out.max_fee_per_gas = Some(max_fee);

        Ok(out)
    }

    /// Attaches or updates the transaction hash for a queued transaction.
    pub fn record_tx_hash(&self, id: &OrderHash, tx_hash: B256) {
        if let Some(mut tx) = self.by_id.get_mut(id) {
            tx.status.tx_hash = Some(tx_hash);
        }
    }

    /// Marks transactions as observed included at the given position if their
    /// hashes match the provided set.
    ///
    /// Returns the `(id, hash)` pairs that matched.
    pub fn record_inclusions(
        &self,
        position: &L2Position,
        tx_hashes: &HashSet<B256>,
    ) -> Vec<(String, TxHash)> {
        let mut out: Vec<(String, B256)> = Vec::new();
        for mut entry in self.by_id.iter_mut() {
            if let Some(h) = entry.status.tx_hash {
                if tx_hashes.contains(&h) {
                    entry.status.observed_position = Some(position.clone());
                    out.push((entry.key().clone(), h));
                }
            }
        }
        out
    }
}
