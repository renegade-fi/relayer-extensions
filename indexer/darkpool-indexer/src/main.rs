//! The darkpool indexer, responsible for maintaining views of committed user
//! state

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use aws_sdk_sqs::types::{Message, MessageSystemAttributeName};
use clap::Parser;
use tokio::task::JoinSet;
use tracing::{error, warn};

use crate::{
    cli::Cli,
    indexer::{Indexer, error::IndexerError},
};

mod api;
mod cli;
mod crypto_mocks;
mod db;
mod indexer;

// -------------
// | Constants |
// -------------

/// The maximum number of messages to receive from SQS
const MAX_RECV_MESSAGES: i32 = 10;

// --------
// | Main |
// --------

#[tokio::main]
async fn main() -> Result<(), IndexerError> {
    let cli = Cli::parse();

    let indexer = Indexer::build_from_cli(&cli).await?;

    let mut tasks = JoinSet::new();
    tasks.spawn(run_relayer_sqs_consumer(indexer.clone(), cli.sqs_queue_url.clone()));

    // TODO: Spawn onchain event SQS consumer & HTTP server
    match tasks.join_next().await.expect("No tasks spawned") {
        Err(e) => error!("Error joining indexer task: {e}"),
        Ok(Ok(())) => warn!("Indexer task exited"),
        Ok(Err(e)) => error!("Indexer task error: {e}"),
    }

    Ok(())
}

/// Run the relayer message SQS consumer, polling for new messages from the
/// relayer and handling them
async fn run_relayer_sqs_consumer(
    indexer: Indexer,
    relayer_sqs_queue_url: String,
) -> Result<(), IndexerError> {
    loop {
        let messages = match indexer
            .sqs_client
            .receive_message()
            .max_number_of_messages(MAX_RECV_MESSAGES)
            .message_system_attribute_names(MessageSystemAttributeName::MessageGroupId)
            .queue_url(&relayer_sqs_queue_url)
            .send()
            .await
        {
            Ok(messages) => messages,
            Err(e) => {
                error!("Error receiving messages from relayer SQS queue: {e}");
                continue;
            },
        };

        // Group messages by message ID.
        // This is necessary because SQS may return multiple messages from multiple
        // message groups in one `receive_message()` call.
        // We want to be sure we processing messages sequentially within a message
        // group, but concurrently across different message groups.
        let mut message_groups: HashMap<String, Vec<Message>> = HashMap::new();
        for message in messages.messages.unwrap_or_default() {
            let message_group_id = message
                .attributes()
                .and_then(|a| a.get(&MessageSystemAttributeName::MessageGroupId).cloned());

            if message_group_id.is_none() {
                warn!(
                    "Message {} from relayer SQS queue has no message group ID, skipping",
                    message.message_id().unwrap_or_default()
                );
                continue;
            }

            message_groups.entry(message_group_id.unwrap()).or_default().push(message);
        }

        // Process message groups concurrently
        for _messages in message_groups.into_values() {
            tokio::spawn(async move {
                // TODO: Message handling logic
            });
        }
    }
}
