//! In-process registry mapping in-flight Fireblocks transaction IDs to webhook
//! waiters.
//!
//! `poll_fireblocks_transaction` subscribes here before it waits on a tx; the
//! `/webhooks/fireblocks/transaction-status` handler dispatches each verified
//! webhook payload to the matching waiter. A terminal-status webhook resolves
//! the waiter in ~milliseconds, so the fallback `get_transaction` poll (now
//! 30s, was 3s) fires zero times in the steady state — which is what removes
//! the poll volume behind the 2026-05-28 Fireblocks overload.
//!
//! Scope: a single process-wide registry, mirroring the global Fireblocks
//! limiter. Fireblocks tx IDs are workspace-global (one API user across all
//! chains) and webhooks arrive on one URL, so a single shared registry is the
//! correct model. Multi-instance routing (`desiredCount > 1`) is out of scope.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use fireblocks_sdk::models::TransactionResponse;
use tokio::sync::broadcast;

/// Per-tx broadcast buffer. One waiter per tx in practice; the small buffer
/// tolerates a burst of status transitions arriving before the waiter polls.
const CHANNEL_CAPACITY: usize = 8;

/// Maps Fireblocks transaction ID → a broadcast sender that fans webhook
/// payloads out to any waiter(s) for that tx.
pub struct TxListenerRegistry {
    /// Live per-tx senders. An entry exists only while at least one waiter is
    /// subscribed; [`TxSubscription`]'s `Drop` prunes idle entries.
    listeners: Mutex<HashMap<String, broadcast::Sender<TransactionResponse>>>,
}

impl TxListenerRegistry {
    /// Construct an empty registry.
    fn new() -> Arc<Self> {
        Arc::new(Self { listeners: Mutex::new(HashMap::new()) })
    }

    /// Subscribe to webhook updates for `tx_id`. Returns a guard holding the
    /// receiver; dropping the guard prunes the registry entry if it was the
    /// last waiter.
    pub fn subscribe(self: &Arc<Self>, tx_id: &str) -> TxSubscription {
        let receiver = {
            let mut listeners = self.listeners.lock().expect("tx listener registry poisoned");
            let sender = listeners
                .entry(tx_id.to_string())
                .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0);
            sender.subscribe()
        };
        TxSubscription {
            registry: Arc::clone(self),
            tx_id: tx_id.to_string(),
            receiver: Some(receiver),
        }
    }

    /// Deliver a webhook payload to the waiter for `payload.id`. Returns `true`
    /// if a live waiter received it; `false` if there's no waiter (the tx isn't
    /// ours, or nobody is awaiting it), in which case the payload is dropped.
    pub fn dispatch(&self, payload: TransactionResponse) -> bool {
        let listeners = self.listeners.lock().expect("tx listener registry poisoned");
        match listeners.get(&payload.id) {
            Some(sender) => sender.send(payload).is_ok(),
            None => false,
        }
    }

    /// Remove the entry for `tx_id` if no receivers remain.
    fn prune(&self, tx_id: &str) {
        let mut listeners = self.listeners.lock().expect("tx listener registry poisoned");
        if listeners.get(tx_id).is_some_and(|s| s.receiver_count() == 0) {
            listeners.remove(tx_id);
        }
    }
}

/// RAII subscription handle. Holds a broadcast receiver and prunes the registry
/// entry on drop when it was the last waiter.
pub struct TxSubscription {
    /// The registry to prune on drop.
    registry: Arc<TxListenerRegistry>,
    /// The tx ID this subscription waits on.
    tx_id: String,
    /// Receiver for webhook payloads dispatched for `tx_id`. `None` only
    /// transiently during `Drop` so the receiver is released before pruning.
    receiver: Option<broadcast::Receiver<TransactionResponse>>,
}

impl TxSubscription {
    /// Await the next webhook payload for this tx. Returns `None` if the
    /// channel closed. `Lagged` (a burst overran the buffer) is skipped —
    /// the fallback poll backstops any genuinely-missed update.
    pub async fn recv(&mut self) -> Option<TransactionResponse> {
        let receiver = self.receiver.as_mut()?;
        loop {
            match receiver.recv().await {
                Ok(payload) => return Some(payload),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl Drop for TxSubscription {
    fn drop(&mut self) {
        // Release our receiver first so `receiver_count()` reflects only any
        // other waiters, then prune the entry if we were the last.
        self.receiver = None;
        self.registry.prune(&self.tx_id);
    }
}

/// Process-wide listener registry, shared by the webhook handler and every
/// `CustodyClient::poll_fireblocks_transaction`. Mirrors the global Fireblocks
/// limiter: one shared instance models the single-workspace tx namespace.
static GLOBAL_TX_LISTENERS: OnceLock<Arc<TxListenerRegistry>> = OnceLock::new();

/// Handle to the process-wide [`TxListenerRegistry`].
pub fn global_tx_listeners() -> Arc<TxListenerRegistry> {
    GLOBAL_TX_LISTENERS.get_or_init(TxListenerRegistry::new).clone()
}
