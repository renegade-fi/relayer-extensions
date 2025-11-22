//! Message queue trait implementation for AWS SQS

use std::marker::PhantomData;

use async_trait::async_trait;
use aws_config::Region;
use aws_sdk_sqs::{Client as SqsClient, types::MessageSystemAttributeName};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::message_queue::{MessageGroupsResponse, MessageQueue, error::MessageQueueError};

// -------------
// | Constants |
// -------------

/// The maximum number of messages to receive from SQS
const MAX_RECV_MESSAGES: i32 = 10;

// ---------------------
// | SQS Message Queue |
// ---------------------

/// A struct wrapping an AWS SQS client for which we implement the abstract
/// message queue trait
pub struct SqsMessageQueue<M> {
    /// The AWS SQS client
    pub sqs_client: SqsClient,
    /// The URL of the AWS SQS queue
    pub sqs_queue_url: String,
    /// The message type supported by the queue
    pub message_type: PhantomData<M>,
}

impl<M> SqsMessageQueue<M> {
    /// Create a new SQS message queue
    pub async fn new(region: String, sqs_queue_url: String) -> Self {
        let config = aws_config::from_env().region(Region::new(region)).load().await;

        let sqs_client = SqsClient::new(&config);

        Self { sqs_client, sqs_queue_url, message_type: PhantomData }
    }
}

#[async_trait]
impl<M: Serialize + for<'de> Deserialize<'de> + Send + Sync> MessageQueue for SqsMessageQueue<M> {
    type Message = M;

    async fn send_message(
        &self,
        message: Self::Message,
        deduplication_id: String,
        message_group: String,
    ) -> Result<(), MessageQueueError> {
        let message_body = serde_json::to_string(&message)?;

        self.sqs_client
            .send_message()
            .queue_url(&self.sqs_queue_url)
            .message_deduplication_id(deduplication_id)
            .message_group_id(message_group)
            .message_body(message_body)
            .send()
            .await
            .map_err(MessageQueueError::send)?;

        Ok(())
    }

    async fn poll_messages(
        &self,
    ) -> Result<MessageGroupsResponse<Self::Message>, MessageQueueError> {
        let messages = self
            .sqs_client
            .receive_message()
            .max_number_of_messages(MAX_RECV_MESSAGES)
            .message_system_attribute_names(MessageSystemAttributeName::MessageGroupId)
            .queue_url(&self.sqs_queue_url)
            .send()
            .await
            .map_err(MessageQueueError::poll)?;

        // Group messages by message ID.
        // This is necessary because SQS may return multiple messages from multiple
        // message groups in one `receive_message()` call.
        // We want to be sure we processing messages sequentially within a message
        // group, but concurrently across different message groups.
        let mut message_groups: MessageGroupsResponse<Self::Message> = MessageGroupsResponse::new();
        for sqs_message in messages.messages.unwrap_or_default() {
            let message_group_id = sqs_message
                .attributes()
                .and_then(|a| a.get(&MessageSystemAttributeName::MessageGroupId).cloned());

            if message_group_id.is_none() {
                warn!(
                    "Message {} from SQS has no message group ID, skipping",
                    sqs_message.message_id().unwrap_or_default()
                );
                continue;
            }

            let message_data =
                message_group_id.zip(sqs_message.body).zip(sqs_message.receipt_handle);

            if let Some(((message_group_id, message_body), receipt_handle)) = message_data {
                let message: Self::Message = serde_json::from_str(&message_body)?;

                message_groups.entry(message_group_id).or_default().push((message, receipt_handle));
            }
        }

        Ok(message_groups)
    }

    async fn delete_message(&self, deletion_id: String) -> Result<(), MessageQueueError> {
        self.sqs_client
            .delete_message()
            .queue_url(&self.sqs_queue_url)
            .receipt_handle(deletion_id)
            .send()
            .await
            .map_err(MessageQueueError::delete)?;

        Ok(())
    }
}
