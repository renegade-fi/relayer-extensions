//! Defines the indexer struct, a dependency injection container which stores
//! handles to shared resources

use alloy::providers::{DynProvider, Provider, ProviderBuilder, WsConnect};
use aws_config::Region;
use aws_sdk_sqs::Client as SqsClient;

use crate::{
    cli::Cli, db::client::DbClient, indexer::error::IndexerError,
    state_transitions::StateApplicator,
};

pub mod error;

/// The indexer struct. Stores handles to shared resources.
#[derive(Clone)]
pub struct Indexer {
    /// The state transition applicator
    pub state_applicator: StateApplicator,
    /// The AWS SQS client
    pub sqs_client: SqsClient,
    /// The WebSocket Ethereum RPC provider
    pub ws_provider: DynProvider,
}

impl Indexer {
    /// Build an indexer from the provided CLI arguments
    pub async fn build_from_cli(cli: &Cli) -> Result<Self, IndexerError> {
        // Set up the database client & state applicator
        let db = DbClient::new(&cli.database_url).await?;
        let state_applicator = StateApplicator::new(db);

        // Set up the AWS SQS client
        let config =
            aws_config::from_env().region(Region::new(cli.sqs_region.clone())).load().await;

        let sqs_client = SqsClient::new(&config);

        // Set up the WebSocket RPC provider
        let ws = WsConnect::new(&cli.ws_rpc_url);
        let ws_provider =
            ProviderBuilder::default().connect_ws(ws).await.map_err(IndexerError::rpc)?.erased();

        // TODO: Parse remaining CLI arguments

        Ok(Self { state_applicator, sqs_client, ws_provider })
    }
}
