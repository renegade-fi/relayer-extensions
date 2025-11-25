//! A mock message queue implementation for testing

use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::message_queue::{MessageGroupsResponse, MessageQueue, error::MessageQueueError};

// ---------
// | Types |
// ---------

/// A message wrapper for the mock message queue
struct MockMessage<M> {
    /// The message
    pub message: M,
    /// The message ID
    /// This is used as the deduplication & deletion ID for the message.
    pub id: String,
    /// Whether the message has already been polled by a consumer.
    /// This determines whether the message should be returned by the
    /// poll_messages method.
    pub polled: bool,
}

impl<M> MockMessage<M> {
    /// Create a new mock message
    fn new(message: M, id: String) -> Self {
        Self { message, id, polled: false }
    }
}

// ----------------------
// | Mock Message Queue |
// ----------------------

/// A mock message queue used for testing. Designed to emulate an AWS SQS FIFO
/// queue w/ an infinite visibility timeout. For more details, see:
/// https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/FIFO-queues-understanding-logic.html#FIFO-receiving-messages
///
/// Utilizes in-memory `VecDeque`s to simulate message groups.
/// The message type for the queues is a tuple of `(message, message_id)`,
/// where the `message_id` is used as the deduplication & deletion ID for the
/// message.
pub struct MockMessageQueue<M> {
    /// A map from message group ID to the queue of messages for that group.
    /// Wrapped in a mutex to allow for simple, thread-safe mutable access.
    message_groups: Mutex<HashMap<String, VecDeque<MockMessage<M>>>>,
}

impl<M> Default for MockMessageQueue<M> {
    /// Create a default mock message queue
    fn default() -> Self {
        Self { message_groups: Mutex::new(HashMap::new()) }
    }
}

// --------------------------------------
// | Message Queue Trait Implementation |
// --------------------------------------

#[async_trait]
impl<M: Serialize + for<'de> Deserialize<'de> + Send + Sync + Clone> MessageQueue
    for MockMessageQueue<M>
{
    type Message = M;

    async fn send_message(
        &self,
        message: Self::Message,
        message_id: String,
        message_group_id: String,
    ) -> Result<(), MessageQueueError> {
        let mut message_groups = self.message_groups.lock().await;

        // Check if the message has already been sent, using the message ID as the
        // deduplication ID
        for (_, message_group) in message_groups.iter() {
            if message_group.iter().any(|message| message.id == message_id) {
                return Ok(());
            }
        }

        let message_group = message_groups.entry(message_group_id.clone()).or_default();
        let mock_message = MockMessage::new(message, message_id);

        message_group.push_back(mock_message);

        Ok(())
    }

    async fn poll_messages(
        &self,
    ) -> Result<MessageGroupsResponse<Self::Message>, MessageQueueError> {
        let mut message_groups = self.message_groups.lock().await;

        let mut message_groups_response = MessageGroupsResponse::new();

        for (message_group_id, message_group) in message_groups.iter_mut() {
            // If any of the messages in the message group have already been polled, no
            // further messages from the group can be returned. This mirrors the
            // behavior of AWS SQS FIFO queues described here: https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/FIFO-queues-understanding-logic.html#FIFO-receiving-messages
            let already_polled = message_group.iter().any(|message| message.polled);
            if already_polled {
                continue;
            }

            let mut messages = vec![];
            for message in message_group.iter_mut() {
                // Copy the group's messages out, and mark them as polled
                messages.push((message.message.clone(), message.id.clone()));
                message.polled = true;
            }

            if !messages.is_empty() {
                message_groups_response.insert(message_group_id.clone(), messages);
            }
        }

        Ok(message_groups_response)
    }

    async fn delete_message(&self, message_id: String) -> Result<(), MessageQueueError> {
        let mut message_groups = self.message_groups.lock().await;
        for (_, message_group) in message_groups.iter_mut() {
            message_group.retain(|message| message.id != message_id);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name for the first testing message group
    const FIRST_MESSAGE_GROUP: &str = "group1";

    /// The name for the second testing message group
    const SECOND_MESSAGE_GROUP: &str = "group2";

    /// Send a vector of unique messages to the given message group
    async fn send_unique_messages<M>(
        queue: &MockMessageQueue<M>,
        messages: Vec<M>,
        message_group: &str,
    ) -> Result<(), MessageQueueError>
    where
        M: Serialize + for<'de> Deserialize<'de> + Send + Sync + Clone + ToString,
    {
        for message in messages {
            let message_id = message.to_string();
            queue.send_message(message, message_id, message_group.to_string()).await?;
        }

        Ok(())
    }

    /// Test the basic send/poll functionality of the mock message queue
    #[tokio::test]
    async fn test_basic_send_poll() {
        let message_queue = MockMessageQueue::default();

        // Send 3 unique messages to the first message group
        send_unique_messages(&message_queue, vec![0, 1, 2], FIRST_MESSAGE_GROUP).await.unwrap();

        // Assert that the same messages are polled from the queue, in the expected
        // order
        let message_groups = message_queue.poll_messages().await.unwrap();
        assert_eq!(message_groups.len(), 1);

        let polled_messages: Vec<i32> = message_groups
            .get(FIRST_MESSAGE_GROUP)
            .unwrap()
            .iter()
            .map(|(message, _)| *message)
            .collect();

        assert_eq!(polled_messages, vec![0, 1, 2]);
    }

    /// Test the message deletion functionality of the mock message queue
    #[tokio::test]
    async fn test_delete_message() {
        let message_queue = MockMessageQueue::default();

        // Send 3 unique messages to the first message group
        send_unique_messages(&message_queue, vec![0, 1, 2], FIRST_MESSAGE_GROUP).await.unwrap();

        // Poll the messages from the queue
        message_queue.poll_messages().await.unwrap();

        // Re-poll the queue, asserting that no messages are returned
        let message_groups = message_queue.poll_messages().await.unwrap();
        assert_eq!(message_groups.len(), 0);

        // Delete the messages from the queue, using their expected message IDs
        message_queue.delete_message("0".to_string()).await.unwrap();
        message_queue.delete_message("1".to_string()).await.unwrap();
        message_queue.delete_message("2".to_string()).await.unwrap();

        // Poll the queue again, asserting that no messages are returned
        let message_groups = message_queue.poll_messages().await.unwrap();
        assert_eq!(message_groups.len(), 0);
    }

    /// Test sending messages to a message group after other messages have been
    /// deleted
    #[tokio::test]
    async fn test_send_after_delete() {
        let message_queue = MockMessageQueue::default();

        // Send 3 unique messages to the first message group
        send_unique_messages(&message_queue, vec![0, 1, 2], FIRST_MESSAGE_GROUP).await.unwrap();

        // Poll the messages from the queue
        message_queue.poll_messages().await.unwrap();

        // Send a new message to the message group
        send_unique_messages(&message_queue, vec![3], FIRST_MESSAGE_GROUP).await.unwrap();

        // Re-poll the queue, asserting that no messages are returned (even though the
        // last one hasn't been polled yet)
        let message_groups = message_queue.poll_messages().await.unwrap();
        assert_eq!(message_groups.len(), 0);

        // Delete the messages from the queue, using their expected message IDs
        message_queue.delete_message("0".to_string()).await.unwrap();
        message_queue.delete_message("1".to_string()).await.unwrap();
        message_queue.delete_message("2".to_string()).await.unwrap();

        // Poll the queue again, asserting that the last message is returned
        let message_groups = message_queue.poll_messages().await.unwrap();
        let polled_messages: Vec<i32> = message_groups
            .get(FIRST_MESSAGE_GROUP)
            .unwrap()
            .iter()
            .map(|(message, _)| *message)
            .collect();

        assert_eq!(polled_messages, vec![3]);
    }

    /// Test that messages can be polled from multiple groups without them
    /// blocking one another
    #[tokio::test]
    async fn test_multiple_message_groups() {
        let message_queue = MockMessageQueue::default();

        // Send 3 unique messages to both message groups
        send_unique_messages(&message_queue, vec![0, 1, 2], FIRST_MESSAGE_GROUP).await.unwrap();
        send_unique_messages(&message_queue, vec![3, 4, 5], SECOND_MESSAGE_GROUP).await.unwrap();

        // Assert that the same messages are polled from the queue, in the expected
        // order
        let message_groups = message_queue.poll_messages().await.unwrap();
        assert_eq!(message_groups.len(), 2);

        let group1_messages: Vec<i32> = message_groups
            .get(FIRST_MESSAGE_GROUP)
            .unwrap()
            .iter()
            .map(|(message, _)| *message)
            .collect();

        assert_eq!(group1_messages, vec![0, 1, 2]);

        let group2_messages: Vec<i32> = message_groups
            .get(SECOND_MESSAGE_GROUP)
            .unwrap()
            .iter()
            .map(|(message, _)| *message)
            .collect();

        assert_eq!(group2_messages, vec![3, 4, 5]);

        // Send a new message to both message groups
        send_unique_messages(&message_queue, vec![6], FIRST_MESSAGE_GROUP).await.unwrap();
        send_unique_messages(&message_queue, vec![7], SECOND_MESSAGE_GROUP).await.unwrap();

        // Delete only the first group's messages
        message_queue.delete_message("0".to_string()).await.unwrap();
        message_queue.delete_message("1".to_string()).await.unwrap();
        message_queue.delete_message("2".to_string()).await.unwrap();

        // Poll the queue again, asserting that only the first group's new message is
        // returned
        let message_groups = message_queue.poll_messages().await.unwrap();
        let group1_messages: Vec<i32> = message_groups
            .get(FIRST_MESSAGE_GROUP)
            .unwrap()
            .iter()
            .map(|(message, _)| *message)
            .collect();

        assert_eq!(group1_messages, vec![6]);

        assert!(!message_groups.contains_key(SECOND_MESSAGE_GROUP))
    }
}
