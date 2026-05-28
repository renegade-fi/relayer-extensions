//! Entry point for the pool-runner service

use std::sync::Arc;

use clap::Parser;
use pool_runner::{
    admin_ws_listener::AdminWebsocketListener,
    cli::Cli,
    log_task,
    logger::{Outcome, Task},
    server::Server,
};
use renegade_config::setup_token_remaps;
use renegade_sdk::{
    ARBITRUM_ONE_CHAIN_ID, ARBITRUM_SEPOLIA_CHAIN_ID, BASE_MAINNET_CHAIN_ID, BASE_SEPOLIA_CHAIN_ID,
    ETHEREUM_SEPOLIA_CHAIN_ID,
};
use renegade_types_core::Chain;
use renegade_util::telemetry::configure_telemetry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Configure logging / tracing / metrics the same way the other v2
    // services do. With `enable_datadog`, logs are emitted as Datadog-format
    // JSON (no ANSI), so the `log_task!` `task` / `outcome` / `subject`
    // fields land as parseable Datadog attributes rather than ANSI-colored
    // pretty text.
    configure_telemetry(
        cli.enable_datadog,
        cli.enable_otlp,
        cli.enable_metrics,
        cli.otlp_collector_endpoint.clone(),
        &cli.statsd_host,
        cli.statsd_port,
    )
    .map_err(|e| anyhow::anyhow!("failed to configure telemetry: {e}"))?;

    // Load the token remap from the canonical token-mappings repo for our
    // chain, then set the process-wide default chain. Without this:
    // - `default_chain()` panics on multi-chain registries (token.rs:374).
    // - `Token::from_ticker("USDC")` panics because the registry is empty.
    // `setup_token_remaps` is sync (uses `reqwest::blocking`), so run it on
    // a blocking thread to avoid stalling the tokio runtime. The helper also
    // calls `set_default_chain` internally, so no separate call is needed.
    let chain = match cli.chain_id {
        ARBITRUM_ONE_CHAIN_ID => Chain::ArbitrumOne,
        ARBITRUM_SEPOLIA_CHAIN_ID => Chain::ArbitrumSepolia,
        BASE_MAINNET_CHAIN_ID => Chain::BaseMainnet,
        BASE_SEPOLIA_CHAIN_ID => Chain::BaseSepolia,
        ETHEREUM_SEPOLIA_CHAIN_ID => Chain::EthereumSepolia,
        other => anyhow::bail!("unknown chain_id: {other}"),
    };
    let _disabled_tickers: Vec<String> =
        tokio::task::spawn_blocking(move || setup_token_remaps(None, chain))
            .await
            .map_err(|e| anyhow::anyhow!("setup_token_remaps join error: {e}"))?
            .map_err(|e| anyhow::anyhow!("setup_token_remaps failed: {e}"))?;

    // Build the server
    let server = Server::build_from_cli(&cli).await?;
    log_task!(
        Task::ServiceLifecycle,
        Outcome::Started,
        subject = "service-boot",
        chain_id = cli.chain_id,
        "Pool runner started on chain_id={}",
        cli.chain_id
    );

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
