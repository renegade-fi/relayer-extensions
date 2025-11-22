//! Defines an abstract interface for a message queue, based on AWS SQS

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::message_queue::error::MessageQueueError;

pub mod error;
pub mod sqs;

// ----------------
// | Type Aliases |
// ----------------

/// A type alias for a map of message groups, containing the messages in the
/// group and their receipt handles
type MessageGroupsResponse<M> = HashMap<String, Vec<(M, String)>>;

// --------------------
// | Trait Definition |
// --------------------

/// A trait describing the high-level interface for a message queue.
///
/// Each message in the queue is expected to be uniquely identified by a
/// deduplication ID, and belong to a "message group." All messages within a
/// group are strictly ordered. However, messages from different groups can be
/// consumed from the queue concurrently.
///
/// These semantics & nomenclature are taken from AWS SQS.
#[async_trait]
pub trait MessageQueue: Sync + Send {
    /// The message type supported by the queue.
    /// We make this an associated type, as opposed to attaching it as a type
    /// parameter to the trait methods, to ensure the trait is
    /// dyn-compatible.
    type Message: Serialize + for<'de> Deserialize<'de> + Send + Sync;

    /// Send a message with the given deduplication ID to the given message
    /// group within the queue
    async fn send_message(
        &self,
        message: Self::Message,
        deduplication_id: String,
        message_group: String,
    ) -> Result<(), MessageQueueError>;

    /// Poll for messages from the queue, collecting them into a map keyed by
    /// message group. Each message is paired with its receipt handle.
    async fn poll_messages(
        &self,
    ) -> Result<MessageGroupsResponse<Self::Message>, MessageQueueError>;

    /// Delete a message from the queue, committing its consumption & ensuring
    /// it will not be polled again
    async fn delete_message(&self, deletion_id: String) -> Result<(), MessageQueueError>;
}

// --------------------------
// | Erased Type Definition |
// --------------------------

/// A type-erased wrapper around a message queue
#[derive(Clone)]
pub struct DynMessageQueue<M>(Arc<dyn MessageQueue<Message = M>>);

impl<M> DynMessageQueue<M> {
    /// Create a new type-erased message queue
    pub fn new<Q: MessageQueue<Message = M> + 'static>(message_queue: Q) -> Self {
        Self(Arc::new(message_queue))
    }
}

#[async_trait]
impl<M: Serialize + for<'de> Deserialize<'de> + Send + Sync> MessageQueue for DynMessageQueue<M> {
    type Message = M;

    async fn send_message(
        &self,
        message: Self::Message,
        deduplication_id: String,
        message_group: String,
    ) -> Result<(), MessageQueueError> {
        self.0.send_message(message, deduplication_id, message_group).await
    }

    async fn poll_messages(
        &self,
    ) -> Result<MessageGroupsResponse<Self::Message>, MessageQueueError> {
        self.0.poll_messages().await
    }

    async fn delete_message(&self, deletion_id: String) -> Result<(), MessageQueueError> {
        self.0.delete_message(deletion_id).await
    }
}
