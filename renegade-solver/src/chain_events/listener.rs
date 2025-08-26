//! Defines a listener for chain events.

use std::collections::HashSet;

use alloy_primitives::keccak256;
use tracing::info;

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
        let fb_hashes: HashSet<_> = fb.diff.transactions.iter().map(keccak256).collect();

        // Mark transactions as observed in the transaction store
        let matches = self.store.record_inclusions(&position, &fb_hashes);

        // Log each match
        for (_id, hash) in matches {
            info!(
                l2_block = position.l2_block,
                observed_fb = position.flashblock,
                hash = %hash,
                "transaction included"
            );
        }
    }
}
