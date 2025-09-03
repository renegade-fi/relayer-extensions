//! Transaction store for L2 transactions.
//!
//! Stores contextual information about fill submissions for multiple workers to
//! access.

use std::collections::HashSet;
use std::sync::Arc;

use alloy_primitives::TxHash;
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

/// Timing information using absolute milliseconds since the UNIX epoch.
#[derive(Clone, Debug)]
pub struct TxTiming {
    /// The absolute milliseconds since the UNIX epoch when we plan to send the
    /// transaction.
    pub send_timestamp_ms: u64,
    /// The absolute milliseconds since the UNIX epoch when we target arrival at
    /// the validator.
    pub target_timestamp_ms: u64,
}

/// Status information captured as the transaction progresses through the chain.
#[derive(Clone, Debug, Default)]
pub struct TxStatus {
    /// The hash of the broadcast transaction (if known).
    pub tx_hash: TxHash,
    /// The observed inclusion position (if seen).
    pub observed_position: Option<L2Position>,
    /// The timestamp when we observed inclusion.
    pub included_timestamp_ms: Option<u64>,
}

/// A pre-computed transaction ready for immediate sending.
#[derive(Clone, Debug)]
pub struct TxContext {
    /// The ID of the transaction. In practice, this is the order hash of the
    /// UniswapX order.
    pub id: OrderHash,
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

    /// Write a transaction context to the store.
    pub fn write(&self, id: &OrderHash, tx_hash: &TxHash, timing: &TxTiming) {
        let tx = TxContext {
            id: id.to_string(),
            timing: timing.clone(),
            status: TxStatus {
                tx_hash: *tx_hash,
                observed_position: None,
                included_timestamp_ms: None,
            },
        };
        self.by_id.insert(tx.id.clone(), tx);
    }

    /// Records observed inclusions and returns context tuples for valid
    /// entries.
    ///
    /// For each transaction whose hash is present in `tx_hashes`, sets
    /// `observed_position` and returns the (target_timestamp_ms,
    /// send_timestamp_ms) tuple.
    pub fn observe_inclusions(
        &self,
        observed: &L2Position,
        included_timestamp_ms: u64,
        hashes: &HashSet<TxHash>,
    ) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        for mut entry in self.by_id.iter_mut() {
            if hashes.contains(&entry.status.tx_hash) {
                entry.status.observed_position = Some(*observed);
                entry.status.included_timestamp_ms = Some(included_timestamp_ms);
                out.push((entry.timing.target_timestamp_ms, entry.timing.send_timestamp_ms));
            }
        }
        out
    }
}
