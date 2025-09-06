//! Transaction store for L2 transactions.
//!
//! Stores contextual information about fill submissions for multiple workers to
//! access.

use std::collections::HashSet;
use std::sync::Arc;

use alloy_primitives::TxHash;
use dashmap::DashMap;

/// Alias for the order hash type.
pub type OrderHash = String;

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
    /// The timestamp when we plan to send the transaction.
    pub send_ts: u64,
    /// The timestamp when we target arrival in the block builder's inbox.
    pub target_ts: u64,
}

/// Status information captured as the transaction progresses through the chain.
#[derive(Clone, Debug, Default)]
pub struct TxStatus {
    /// The hash of the broadcast transaction (if known).
    pub tx_hash: TxHash,
    /// The observed inclusion position (if seen).
    pub observed_position: Option<L2Position>,
    /// The timestamp when we observed inclusion.
    pub included_ts: Option<u64>,
    /// The timestamp when we submitted the transaction.
    pub submitted_ts: Option<u64>,
}

/// A pre-computed transaction ready for immediate sending.
#[derive(Clone, Debug)]
pub struct TxContext {
    /// The ID of the transaction. In practice, this is the order hash of the
    /// UniswapX order.
    pub id: OrderHash,
    /// The position we target for inclusion.
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

impl Default for TxStore {
    fn default() -> Self {
        Self { by_id: Arc::new(DashMap::new()) }
    }
}

impl TxStore {
    // --------------
    // | Public API |
    // --------------

    /// Write a transaction context to the store.
    pub fn write(&self, id: &OrderHash, tx_hash: &TxHash, timing: &TxTiming, target: &L2Position) {
        let tx = TxContext {
            id: id.to_string(),
            target: *target,
            timing: timing.clone(),
            status: TxStatus {
                tx_hash: *tx_hash,
                observed_position: None,
                included_ts: None,
                submitted_ts: None,
            },
        };
        self.by_id.insert(tx.id.clone(), tx);
    }

    /// Record the timestamp when we submitted the transaction.
    pub fn record_submission(&self, id: &OrderHash, submitted_ts: u64) {
        self.by_id.get_mut(id).unwrap().status.submitted_ts = Some(submitted_ts);
    }

    /// Records observed inclusions and returns context tuples for valid
    /// entries.
    ///
    /// For each transaction whose hash is present in `tx_hashes`, sets
    /// `observed_position` and `included_ts`.
    ///
    /// Returns the transaction context for each valid entry.
    pub fn read_by_hashes(
        &self,
        hashes: &HashSet<TxHash>,
        observed: &L2Position,
        included_ts: u64,
    ) -> Vec<TxContext> {
        let mut out: Vec<TxContext> = Vec::new();
        for mut entry in self.by_id.iter_mut() {
            if hashes.contains(&entry.status.tx_hash) {
                entry.status.observed_position = Some(*observed);
                entry.status.included_ts = Some(included_ts);
                out.push(entry.clone());
            }
        }
        out
    }
}
