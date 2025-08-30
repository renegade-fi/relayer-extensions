//! Triggered transaction store for L2 transactions.
//!
//! Tracks queued pre-computed transactions keyed by ID, the L2 trigger at which
//! they should be sent, and their inclusion status. All expensive operations
//! (hydration, signing) are performed at insertion time.

use std::collections::HashSet;
use std::sync::Arc;

use alloy_primitives::{Bytes, TxHash};

use crate::tx_store::error::TxStoreResult;
use dashmap::DashMap;

/// Alias for the order hash type.
type OrderHash = String;

/// A position on the L2 chain (block and flashblock).
#[derive(Clone, Debug, PartialEq, Eq, Copy)]
pub struct L2Position {
    /// The L2 block number.
    pub l2_block: u64,
    /// The flashblock number.
    pub flashblock: u64,
}

impl L2Position {
    /// Returns the linearized flashblock index for this position given the
    /// flashblocks-per-block value.
    pub fn linear_index(&self, flashblocks_per_block: u64) -> u128 {
        (self.l2_block as u128) * (flashblocks_per_block as u128) + (self.flashblock as u128)
    }

    /// Constructs an `L2Position` from a linearized flashblock index.
    pub fn from_linear_index(idx: u128, flashblocks_per_block: u64) -> Self {
        let fpb = flashblocks_per_block.max(1) as u128;
        let l2_block = (idx / fpb) as u64;
        let flashblock = (idx % fpb) as u64;
        Self { l2_block, flashblock }
    }
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
    pub tx_hash: TxHash,
    /// The observed inclusion position (if seen).
    pub observed_position: Option<L2Position>,
}

/// A pre-computed transaction ready for immediate sending.
#[derive(Clone, Debug)]
pub struct TxContext {
    /// The ID of the transaction. In practice, this is the order hash of the
    /// UniswapX order.
    pub id: OrderHash,
    /// The pre-signed raw transaction bytes.
    pub raw_tx_bytes: Bytes,
    /// The target position for which we aim the inclusion.
    pub target: L2Position,
    /// The timing information for when to send.
    pub timing: TxTiming,
    /// The evolving status of this transaction.
    pub status: TxStatus,
}

/// A thread-safe store for pre-computed transactions.
#[derive(Clone)]
pub struct TxStore {
    /// The inner store of pre-computed transactions.
    by_id: Arc<DashMap<OrderHash, TxContext>>,
}

impl TxStore {
    /// Creates a new `TxStore`.
    pub fn new() -> Self {
        Self { by_id: Arc::new(DashMap::new()) }
    }

    // --------------
    // | Public API |
    // --------------

    /// Enqueues a pre-computed transaction.
    pub fn enqueue(
        &self,
        id: &OrderHash,
        raw_tx_bytes: Bytes,
        tx_hash: TxHash,
        target: L2Position,
        timing: TxTiming,
    ) {
        let tx = TxContext {
            id: id.to_string(),
            raw_tx_bytes,
            target,
            timing,
            status: TxStatus { tx_hash, observed_position: None },
        };
        self.by_id.insert(tx.id.clone(), tx);
    }

    /// Returns the transactions that are due to send at the specified trigger
    /// position, including payloads needed for sending.
    pub fn due_at(&self, at: &L2Position) -> Vec<(OrderHash, u64, Bytes, TxHash)> {
        self.by_id
            .iter()
            .filter_map(|entry| {
                let id = entry.key();
                let tx = entry.value();
                if tx.timing.triggers_at(at) {
                    Some((
                        id.clone(),
                        tx.timing.buffer_ms,
                        tx.raw_tx_bytes.clone(),
                        tx.status.tx_hash,
                    ))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Records observed inclusions and returns context tuples for valid
    /// entries.
    ///
    /// For each transaction whose hash is present in `tx_hashes`, sets
    /// `observed_position` and returns the (id, target, observed) tuple.
    pub fn observe_inclusions(
        &self,
        observed: &L2Position,
        hashes: &HashSet<TxHash>,
    ) -> Vec<(String, L2Position, L2Position)> {
        let mut out: Vec<(String, L2Position, L2Position)> = Vec::new();
        for mut entry in self.by_id.iter_mut() {
            let id = entry.key().clone();
            if hashes.contains(&entry.status.tx_hash) {
                entry.status.observed_position = Some(*observed);
                out.push((id, entry.target, *observed));
            }
        }
        out
    }
}
