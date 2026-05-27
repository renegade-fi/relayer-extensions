//! Fill-waiter registry: tracks pending fills for in-flight match attempts.
//!
//! When the pool runner assigns a user order to a managed pool, it registers
//! a oneshot channel here keyed by the order ID. The admin WS listener fires
//! the channel when it sees a fill update for that order.

use std::collections::HashMap;

use renegade_external_api::types::AdminOrderUpdateMessage;
use tokio::sync::{RwLock, oneshot};
use uuid::Uuid;

/// Registry of fill waiters keyed by order ID
pub struct FillWaiterRegistry {
    waiters: RwLock<HashMap<Uuid, oneshot::Sender<AdminOrderUpdateMessage>>>,
}

impl FillWaiterRegistry {
    pub fn new() -> Self {
        Self { waiters: RwLock::new(HashMap::new()) }
    }

    /// Register a fill waiter for the given order. Returns the receiver end.
    pub async fn register(&self, order_id: Uuid) -> oneshot::Receiver<AdminOrderUpdateMessage> {
        let (tx, rx) = oneshot::channel();
        self.waiters.write().await.insert(order_id, tx);
        rx
    }

    /// Notify the waiter for the given order, if one is registered.
    /// Returns `true` if a waiter was found and notified.
    pub async fn notify(&self, order_id: Uuid, message: AdminOrderUpdateMessage) -> bool {
        if let Some(tx) = self.waiters.write().await.remove(&order_id) {
            let _ = tx.send(message);
            true
        } else {
            false
        }
    }

    /// Remove a waiter without notifying (cleanup after timeout).
    pub async fn remove(&self, order_id: Uuid) {
        self.waiters.write().await.remove(&order_id);
    }
}

impl Default for FillWaiterRegistry {
    fn default() -> Self {
        Self::new()
    }
}
