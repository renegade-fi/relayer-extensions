//! Server struct, startup, and HTTP healthcheck for the pool-runner service
//!
//! Detects user orders in the global matching pool and attempts to route them
//! into a managed MM pool, awaiting a fill before reassigning back to the
//! global pool.

mod runner;

use std::sync::Arc;

use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result};
use price_reporter_client::{PriceReporterClient, PriceReporterClientConfig};
use renegade_constants::GLOBAL_MATCHING_POOL;
use renegade_sdk::client::RenegadeClient;
use renegade_types_core::HmacKey;
use warp::Filter;

use crate::{
    cli::Cli,
    config::{RunnerConfig, load_runner_config},
    fill_waiter::FillWaiterRegistry,
    log_task,
    logger::{Outcome, Task},
};

/// Central server state, shared across tasks via `Arc<Server>`
pub struct Server {
    /// Admin client for relayer API calls
    pub(crate) admin_client: RenegadeClient,
    /// Price reporter client for fetching token prices
    pub(crate) price_reporter: Arc<PriceReporterClient>,
    /// Runner configuration (managed pools)
    pub(crate) config: RunnerConfig,
    /// Fill-waiter registry for in-flight match attempts
    pub(crate) fill_waiters: FillWaiterRegistry,
}

impl Server {
    /// Build a `Server` from parsed CLI arguments
    pub async fn build_from_cli(cli: &Cli) -> Result<Arc<Self>> {
        let config = load_runner_config(&cli.runner_config_path)
            .await
            .context("Failed to load runner config")?;

        let admin_client = build_admin_client(&cli.relayer_admin_key, cli.chain_id)?;

        let price_reporter_config = PriceReporterClientConfig {
            base_url: cli.price_reporter_url.clone(),
            ..Default::default()
        };
        let price_reporter = Arc::new(
            PriceReporterClient::new(price_reporter_config)
                .context("Failed to build price reporter client")?,
        );

        Ok(Arc::new(Self {
            admin_client,
            price_reporter,
            config,
            fill_waiters: FillWaiterRegistry::new(),
        }))
    }

    /// Fetch all open orders in the global pool and spawn a runner task for
    /// each eligible user order.
    pub async fn process_open_orders(self: &Arc<Self>) -> Result<()> {
        log_task!(
            Task::PoolRouter,
            Outcome::Started,
            subject = "fetch-open-orders",
            "Fetching open orders in global matching pool"
        );

        let global_orders = self
            .admin_client
            .admin_get_open_orders_in_matching_pool(GLOBAL_MATCHING_POOL.to_string())
            .await
            .context("Failed to fetch global pool orders")?;

        log_task!(
            Task::PoolRouter,
            Outcome::Ok,
            subject = "fetch-open-orders",
            order_count = global_orders.len(),
            "Found {} orders in global matching pool",
            global_orders.len()
        );

        for order in global_orders {
            let server = self.clone();
            tokio::spawn(async move {
                if let Err(e) = server.try_route_user_order(&order).await {
                    log_task!(
                        Task::PoolRouter,
                        Outcome::Failed,
                        subject = "route-order",
                        order_id = %order.order.id,
                        "Failed to route order {}: {e}",
                        order.order.id
                    );
                }
            });
        }

        Ok(())
    }

    /// Run the HTTP healthcheck server on the configured port.
    pub async fn run_healthcheck(port: u16) {
        let route = warp::path("healthcheck")
            .and(warp::get())
            .map(|| warp::reply::json(&serde_json::json!({"status": "ok"})));

        log_task!(
            Task::HttpServer,
            Outcome::Started,
            subject = "healthcheck-listen",
            port = port,
            "Starting healthcheck server on port {port}"
        );
        warp::serve(route).run(([0, 0, 0, 0], port)).await;
    }
}

/// Build an admin RenegadeClient for the given HMAC key and chain ID
fn build_admin_client(relayer_admin_key: &str, chain_id: u64) -> Result<RenegadeClient> {
    use renegade_sdk::{
        ARBITRUM_ONE_CHAIN_ID, ARBITRUM_SEPOLIA_CHAIN_ID, BASE_MAINNET_CHAIN_ID,
        BASE_SEPOLIA_CHAIN_ID, ETHEREUM_SEPOLIA_CHAIN_ID,
    };

    let hmac_key = HmacKey::from_base64_string(relayer_admin_key)
        .map_err(|e| anyhow::anyhow!("Invalid relayer admin key: {e}"))?;

    // A dummy signer is sufficient — admin clients never sign wallet actions
    let dummy_signer = PrivateKeySigner::random();

    let client = match chain_id {
        ARBITRUM_SEPOLIA_CHAIN_ID => {
            RenegadeClient::new_arbitrum_sepolia_admin(&dummy_signer, hmac_key.into())
        },
        ARBITRUM_ONE_CHAIN_ID => {
            RenegadeClient::new_arbitrum_one_admin(&dummy_signer, hmac_key.into())
        },
        BASE_SEPOLIA_CHAIN_ID => {
            RenegadeClient::new_base_sepolia_admin(&dummy_signer, hmac_key.into())
        },
        BASE_MAINNET_CHAIN_ID => {
            RenegadeClient::new_base_mainnet_admin(&dummy_signer, hmac_key.into())
        },
        ETHEREUM_SEPOLIA_CHAIN_ID => {
            RenegadeClient::new_ethereum_sepolia_admin(&dummy_signer, hmac_key.into())
        },
        _ => anyhow::bail!("Unsupported chain ID: {chain_id}"),
    };

    client.context("Failed to build admin client")
}
