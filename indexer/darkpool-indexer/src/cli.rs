//! Command-line interface for the darkpool indexer

use clap::Parser;
use renegade_common::types::chain::Chain;

/// The darkpool indexer CLI
#[rustfmt::skip]
#[derive(Parser)]
#[clap(about = "Darkpool indexer")]
pub struct Cli {
    // ------------
    // | Database |
    // ------------

    /// The database URL
    #[clap(long, env = "DATABASE_URL")]
    pub database_url: String,

    // ---------------
    // | HTTP Server |
    // ---------------

    /// The port to run the HTTP server on
    #[clap(long, default_value = "3000")]
    pub http_port: u16,
    /// The authentication key for the HTTP server, base64-encoded.
    /// 
    /// If not provided, the HTTP server will not be authenticated.
    #[clap(long, env = "HTTP_AUTH_KEY")]
    pub auth_key: Option<String>,

    // -----------
    // | AWS SQS |
    // -----------

    /// The URL of the AWS SQS queue
    #[clap(long, env = "SQS_QUEUE_URL")]
    pub sqs_queue_url: String,
    /// The AWS region in which the SQS queue is located
    #[clap(long, env = "SQS_REGION", default_value = "us-east-2")]
    pub sqs_region: String,

    // --------------
    // | Blockchain |
    // --------------

    /// The chain for which to index darkpool state
    #[clap(long, env = "CHAIN")]
    pub chain: Chain,
    /// The JSON-RPC URL to use for blockchain interaction
    #[clap(long, env = "JSON_RPC_URL")]
    pub json_rpc_url: String,
    /// The Websocket RPC URL to use for listening to onchain events
    #[clap(long, env = "WS_RPC_URL")]
    pub ws_rpc_url: String,
    #[clap(long, env = "DARKPOOL_ADDRESS")]
    /// The address of the darkpool contract
    pub darkpool_address: String,

    // -------------
    // | Telemetry |
    // -------------

    /// Whether or not to forward telemetry to Datadog
    #[clap(long, env = "ENABLE_DATADOG")]
    pub datadog_enabled: bool,
}
