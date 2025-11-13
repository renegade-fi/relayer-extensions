//! The darkpool indexer, responsible for maintaining views of committed user
//! state

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]
#![feature(let_chains)]

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
mod darkpool_client;
mod db;
mod indexer;
mod state_transitions;
mod types;

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
    tasks.spawn(run_sqs_consumer(indexer.clone(), cli.sqs_queue_url.clone()));

    // TODO: Spawn HTTP server
    match tasks.join_next().await.expect("No tasks spawned") {
        Err(e) => error!("Error joining indexer task: {e}"),
        Ok(Ok(())) => warn!("Indexer task exited"),
        Ok(Err(e)) => error!("Indexer task error: {e}"),
    }

    Ok(())
}

/// Run the SQS consumer, polling for new messages from the
/// queue and handling them
async fn run_sqs_consumer(indexer: Indexer, sqs_queue_url: String) -> Result<(), IndexerError> {
    loop {
        let messages = match indexer
            .sqs_client
            .receive_message()
            .max_number_of_messages(MAX_RECV_MESSAGES)
            .message_system_attribute_names(MessageSystemAttributeName::MessageGroupId)
            .queue_url(&sqs_queue_url)
            .send()
            .await
        {
            Ok(messages) => messages,
            Err(e) => {
                error!("Error receiving messages from SQS: {e}");
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
                    "Message {} from SQS has no message group ID, skipping",
                    message.message_id().unwrap_or_default()
                );
                continue;
            }

            message_groups.entry(message_group_id.unwrap()).or_default().push(message);
        }

        // Process message groups concurrently
        for messages in message_groups.into_values() {
            let indexer_clone = indexer.clone();
            let sqs_queue_url_clone = sqs_queue_url.clone();
            tokio::spawn(async move {
                // Process messages within a message group sequentially
                for message in messages {
                    if let Err(e) =
                        indexer_clone.handle_sqs_message(message, &sqs_queue_url_clone).await
                    {
                        error!("Error handling SQS message: {e}")
                    }
                }
            });
        }
    }
}
