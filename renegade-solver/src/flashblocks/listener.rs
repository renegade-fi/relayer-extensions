#![allow(missing_docs, clippy::missing_docs_in_private_items, unused)]
//! Subscribes to flashblocks events and forwards them to the
//! `FlashblocksReceiver` trait.
//!
//! This is copied from https://github.com/base/node-reth/blob/main/crates/flashblocks-rpc/src/subscription.rs
//! to avoid the dependency on `node-reth`.

use std::time::Instant;
use std::{io::Read, sync::Arc};

use alloy_primitives::map::foldhash::HashMap;
use alloy_primitives::{Address, U256};
use alloy_rpc_types_engine::PayloadId;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info};
use url::Url;

use crate::flashblocks::types::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashblocksPayloadV1,
};

/// A trait for receiving flashblocks.
pub trait FlashblocksReceiver {
    fn on_flashblock_received(&self, flashblock: Flashblock);
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Metadata {
    // If this field is needed in the future, add the `reth_optimism_primitives` dependency
    // and use the `OpReceipt` type.
    // There were rustc combatibility issues with `ark_mpc` so it was omitted for now.
    // pub receipts: HashMap<B256, OpReceipt>,
    pub new_account_balances: HashMap<Address, U256>,
    pub block_number: u64,
}

#[derive(Debug, Clone)]
/// A flashblock received from the flashblocks websocket.
pub struct Flashblock {
    pub payload_id: PayloadId,
    pub index: u64,
    pub base: Option<ExecutionPayloadBaseV1>,
    pub diff: ExecutionPayloadFlashblockDeltaV1,
    pub metadata: Metadata,
    pub received_at: Instant,
}

/// Simplify actor messages to just handle shutdown.
#[derive(Debug)]
enum ActorMessage {
    BestPayload { payload: Flashblock },
}

/// A subscriber that listens for flashblocks and forwards them to a receiver.
pub struct FlashblocksSubscriber<Receiver> {
    flashblocks_state: Arc<Receiver>,
    ws_url: Url,
}

impl<Receiver> FlashblocksSubscriber<Receiver>
where
    Receiver: FlashblocksReceiver + Send + Sync + 'static,
{
    /// Creates a new `FlashblocksSubscriber` with the given receiver and
    /// websocket URL.
    pub fn new(flashblocks_state: Arc<Receiver>, ws_url: Url) -> Self {
        Self { ws_url, flashblocks_state }
    }

    /// Starts the subscriber.
    pub fn start(self) {
        info!(
            message = "Starting Flashblocks subscription",
            url = %self.ws_url,
        );

        let ws_url = self.ws_url.clone();

        let (sender, mut mailbox) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut backoff = std::time::Duration::from_secs(1);
            const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(10);

            loop {
                match connect_async(ws_url.as_str()).await {
                    Ok((ws_stream, _)) => {
                        info!(message = "WebSocket connection established");

                        let (_, mut read) = ws_stream.split();

                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(Message::Binary(bytes)) => match try_decode_message(&bytes) {
                                    Ok(payload) => {
                                        let _ = sender.send(ActorMessage::BestPayload { payload: payload.clone() }).await.map_err(|e| {
                                            error!(message = "Failed to publish message to channel", error = %e);
                                        });
                                    },
                                    Err(e) => {
                                        error!(
                                            message = "error decoding flashblock message",
                                            error = %e
                                        );
                                    },
                                },
                                Ok(Message::Close(_)) => {
                                    info!(message = "WebSocket connection closed by upstream");
                                    break;
                                },
                                Err(e) => {
                                    error!(
                                        message = "error receiving message",
                                        error = %e
                                    );
                                    break;
                                },
                                _ => {},
                            }
                        }
                    },
                    Err(e) => {
                        error!(
                            message = "WebSocket connection error, retrying",
                            backoff_duration = ?backoff,
                            error = %e
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
                        continue;
                    },
                }
            }
        });

        let flashblocks_state = self.flashblocks_state.clone();
        tokio::spawn(async move {
            while let Some(message) = mailbox.recv().await {
                match message {
                    ActorMessage::BestPayload { payload } => {
                        flashblocks_state.on_flashblock_received(payload);
                    },
                }
            }
        });
    }
}

/// Decodes a flashblock message and returns a `Flashblock` struct.
fn try_decode_message(bytes: &[u8]) -> eyre::Result<Flashblock> {
    let text = try_parse_message(bytes)?;

    let payload: FlashblocksPayloadV1 = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            return Err(eyre::eyre!("failed to parse message: {}", e));
        },
    };

    let metadata: Metadata = match serde_json::from_value(payload.metadata.clone()) {
        Ok(m) => m,
        Err(e) => {
            return Err(eyre::eyre!("failed to parse message metadata: {}", e));
        },
    };

    Ok(Flashblock {
        payload_id: payload.payload_id,
        index: payload.index,
        base: payload.base,
        diff: payload.diff,
        metadata,
        received_at: Instant::now(),
    })
}

/// Parses a brotli-compressed message.
fn try_parse_message(bytes: &[u8]) -> eyre::Result<String> {
    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
        if text.trim_start().starts_with("{") {
            return Ok(text);
        }
    }

    let mut decompressor = brotli::Decompressor::new(bytes, 4096);
    let mut decompressed = Vec::new();
    decompressor.read_to_end(&mut decompressed)?;

    let text = String::from_utf8(decompressed)?;
    Ok(text)
}
