//! Defines a listener for chain events.

use std::collections::HashSet;

use alloy_primitives::keccak256;

use crate::arrival_control::controller::ArrivalController;
use crate::flashblocks::clock::get_current_time_millis;
use crate::flashblocks::{Flashblock, FlashblocksReceiver};
use crate::tx_store::store::{L2Position, TxStore};

/// The listener for chain events.
#[derive(Clone)]
pub struct ChainEventsListener {
    /// The transaction store
    store: TxStore,
    /// Arrival controller (RTT-only feedback in epoch ms)
    controller: ArrivalController,
}

impl ChainEventsListener {
    /// Creates a new `ChainEventsListener` with the given store.
    pub fn new(store: TxStore, controller: ArrivalController) -> Self {
        Self { store, controller }
    }
}

impl FlashblocksReceiver for ChainEventsListener {
    fn on_flashblock_received(&self, fb: Flashblock) {
        let position = L2Position {
            l2_block: fb.metadata.block_number,
            flashblock: fb.index,
        };

        // List of hashes reported in this flashblock's diff
        let fb_hashes: HashSet<_> = fb.diff.transactions.iter().map(keccak256).collect();

        // Mark transactions as observed and capture epoch millis for inclusion.
        let included_timestamp_ms = get_current_time_millis();

        let observed_tuples =
            self.store
                .observe_inclusions(&position, included_timestamp_ms, &fb_hashes);

        // Feed back update for each observed transaction.
        for (target_timestamp_ms, sent_at_timestamp_ms) in observed_tuples {
            self.controller.on_feedback(
                target_timestamp_ms,
                sent_at_timestamp_ms,
                included_timestamp_ms,
            );
            tracing::info!(
                "observed - target: {}ms",
                included_timestamp_ms as f64 - target_timestamp_ms as f64
            );
            tracing::info!(
                "observed inclusion in {}#{}",
                position.l2_block,
                position.flashblock,
            );
        }
    }
}
