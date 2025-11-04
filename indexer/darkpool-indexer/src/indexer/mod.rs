//! Defines the indexer struct, a dependency injection container which stores
//! handles to shared resources

use aws_config::Region;
use aws_sdk_sqs::Client as SqsClient;

use crate::{cli::Cli, db::client::DbClient, indexer::error::IndexerError};

pub mod error;

/// The indexer struct. Stores handles to shared resources.
#[derive(Clone)]
pub struct Indexer {
    /// The database client
    pub db: DbClient,
    /// The AWS SQS client
    pub sqs_client: SqsClient,
}

impl Indexer {
    /// Build an indexer from the provided CLI arguments
    pub async fn build_from_cli(cli: &Cli) -> Result<Self, IndexerError> {
        // Set up the database client
        let db = DbClient::new(&cli.database_url).await?;

        // Set up the AWS SQS client
        let config =
            aws_config::from_env().region(Region::new(cli.sqs_region.clone())).load().await;

        let sqs_client = SqsClient::new(&config);

        // TODO: Parse remaining CLI arguments

        Ok(Self { db, sqs_client })
    }
}
