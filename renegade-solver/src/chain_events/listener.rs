//! Defines a listener for chain events.

use std::collections::HashSet;

use alloy_primitives::keccak256;

use crate::flashblocks::{Flashblock, FlashblocksReceiver};
use crate::tx_store::store::{L2Position, TxStore};

/// The listener for chain events.
#[derive(Clone)]
pub struct ChainEventsListener {
    /// The transaction store
    store: TxStore,
}

impl ChainEventsListener {
    /// Creates a new `ChainEventsListener` with the given store.
    pub fn new(store: TxStore) -> Self {
        Self { store }
    }
}

impl FlashblocksReceiver for ChainEventsListener {
    fn on_flashblock_received(&self, fb: Flashblock) {
        let position = L2Position { l2_block: fb.metadata.block_number, flashblock: fb.index };

        // List of hashes reported in this flashblock's diff
        let hashes: HashSet<_> = fb.diff.transactions.iter().map(keccak256).collect();

        // Mark transactions as observed and capture epoch millis for inclusion.
        let actual_ts = fb.received_at;

        let observed_tuples = self.store.read_by_hashes(&hashes, &position, actual_ts);

        // Feed back update for each observed transaction.
        for tx in observed_tuples {
            let predicted_ts = tx.timing.target_ts;
            let expected_block = tx.target.l2_block;
            let expected_flashblock = tx.target.flashblock;
            tracing::info!("actual - predicted: {}ms", actual_ts as f64 - predicted_ts as f64);
            tracing::info!("expected {}#{}", expected_block, expected_flashblock);
            tracing::info!("got {}#{}", position.l2_block, position.flashblock,);
        }
    }
}
