//! Entry point for the pool-runner service

use std::sync::Arc;

use clap::Parser;
use tracing::info;

use pool_runner::{admin_ws_listener::AdminWebsocketListener, cli::Cli, server::Server};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    // Build the server
    let server = Server::build_from_cli(&cli).await?;
    info!("Pool runner started on chain_id={}", cli.chain_id);

    // Process any orders already in the global pool at startup
    server.process_open_orders().await?;

    // Spawn the admin WS listener
    let listener = Arc::new(AdminWebsocketListener::new(
        &cli.relayer_admin_key,
        cli.chain_id,
        server.clone(),
    )?);

    let listener_clone = listener.clone();
    tokio::spawn(async move {
        listener_clone.listen().await;
    });

    // Run the HTTP healthcheck (blocks forever)
    Server::run_healthcheck(cli.port).await;

    Ok(())
}
