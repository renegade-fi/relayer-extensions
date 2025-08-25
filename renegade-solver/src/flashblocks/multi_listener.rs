//! Defines a composite listener that fans out to multiple listeners.

use std::sync::Arc;

use crate::flashblocks::listener::{Flashblock, FlashblocksReceiver};

/// The composite listener
pub struct MultiListener {
    /// The listeners to forward the flashblocks to.
    listeners: Vec<Arc<dyn FlashblocksReceiver + Send + Sync>>,
}

impl MultiListener {
    /// Creates a new `MultiListener` with the given listeners.
    pub fn new(listeners: Vec<Arc<dyn FlashblocksReceiver + Send + Sync>>) -> Self {
        Self { listeners }
    }
}

impl FlashblocksReceiver for MultiListener {
    fn on_flashblock_received(&self, flashblock: Flashblock) {
        for listener in &self.listeners {
            // Clone so each listener gets its own owned copy
            listener.on_flashblock_received(flashblock.clone());
        }
    }
}
