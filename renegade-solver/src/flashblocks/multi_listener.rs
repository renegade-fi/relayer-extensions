//! Defines a composite listener that fans out to multiple listeners and owns
//! the WebSocket subscription lifecycle.

use std::sync::Arc;

use url::Url;

use crate::cli::Cli;
use crate::flashblocks::listener::{Flashblock, FlashblocksReceiver, FlashblocksSubscriber};

/// The public listener that routes Flashblocks events to multiple receivers
pub struct FlashblocksListener {
    /// The listeners to forward the flashblocks to.
    listeners: Vec<Box<dyn FlashblocksReceiver + Send + Sync>>,
    /// The websocket URL for flashblocks
    ws_url: Url,
}

impl FlashblocksListener {
    /// Creates a new `MultiListener` with the given listeners and CLI config.
    pub fn new(listeners: Vec<Box<dyn FlashblocksReceiver + Send + Sync>>, cli: &Cli) -> Self {
        let ws_url =
            Url::parse(&cli.fb_websocket_url).expect("Failed to parse flashblocks websocket url");
        Self { listeners, ws_url }
    }

    /// Start the flashblocks subscription
    pub fn start(self) {
        let ws_url = self.ws_url.clone();
        let subscriber = FlashblocksSubscriber::new(Arc::new(self), ws_url);
        subscriber.start();
    }
}

impl FlashblocksReceiver for FlashblocksListener {
    fn on_flashblock_received(&self, flashblock: Flashblock) {
        for listener in &self.listeners {
            // Clone so each listener gets its own owned copy
            listener.on_flashblock_received(flashblock.clone());
        }
    }
}
