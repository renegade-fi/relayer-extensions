//! UniswapX API client and handlers

use std::{collections::HashMap, sync::Arc, time::Duration};

use lru::LruCache;
use renegade_sdk::ExternalMatchClient;
use reqwest::Client as ReqwestClient;
use tokio::sync::RwLock;
use tracing::error;

use crate::{cli::Cli, error::SolverResult, uniswapx::api_types::OrderEntity};

mod api_interaction;
mod api_types;
mod solve;

/// The interval at which to poll for new orders
const POLLING_INTERVAL: Duration = Duration::from_secs(1);

/// The maximum number of orders to cache
///
/// Orders are typically only duplicated for a short window while the auction
/// commences, so a small cache is sufficient.
const ORDER_CACHE_SIZE: usize = 100;

/// A shared read-only hashmap of supported tokens
///
/// Maps from address to symbol
type SupportedTokens = Arc<HashMap<String, String>>;
/// A shared read-only LRU cache of order hashes we've already tried to handle
type OrderCache = Arc<RwLock<LruCache<String, ()>>>;

/// Create a new order cache
fn new_order_cache() -> OrderCache {
    let cache_size = std::num::NonZeroUsize::new(ORDER_CACHE_SIZE).unwrap();
    Arc::new(RwLock::new(LruCache::new(cache_size)))
}

/// The UniswapX API client
#[derive(Clone)]
pub struct UniswapXSolver {
    /// The base URL for the UniswapX API
    base_url: String,
    /// The set of known tokens
    ///
    /// Maps from address to symbol
    supported_tokens: SupportedTokens,
    /// The API client
    http_client: ReqwestClient,
    /// The Renegade client
    renegade_client: ExternalMatchClient,
    /// LRU cache of order hashes we've already tried to handle
    ///
    /// An order is placed in the cache even if processing the order fails, this
    /// cache is for deduplicating requests rather than tracking order status.
    order_cache: OrderCache,
}

impl UniswapXSolver {
    // ---------
    // | Setup |
    // ---------

    /// Create a new UniswapX solver
    pub async fn new(cli: Cli) -> SolverResult<Self> {
        let Cli { uniswapx_url: base_url, renegade_api_key, renegade_api_secret, .. } = cli;

        // TODO: Add support for other chains
        let renegade_client =
            ExternalMatchClient::new_base_mainnet_client(&renegade_api_key, &renegade_api_secret)?;
        let supported_tokens = Self::load_supported_tokens(&renegade_client).await?;

        Ok(Self {
            base_url,
            http_client: ReqwestClient::new(),
            renegade_client,
            supported_tokens,
            order_cache: new_order_cache(),
        })
    }

    /// Load the known tokens from the database
    async fn load_supported_tokens(client: &ExternalMatchClient) -> SolverResult<SupportedTokens> {
        let resp = client.get_supported_tokens().await?;
        let mut map = HashMap::with_capacity(resp.tokens.len());
        for token in resp.tokens {
            map.insert(token.address.to_lowercase(), token.symbol);
        }

        Ok(Arc::new(map))
    }

    // -----------
    // | Helpers |
    // -----------

    /// Check if a token is supported
    async fn is_token_supported(&self, token: &str) -> bool {
        let token = token.to_lowercase();
        self.supported_tokens.contains_key(&token)
    }

    /// Check if an order has already been processed
    async fn is_order_processed(&self, order: &OrderEntity) -> bool {
        let hash = order.order_hash.clone();
        let cache = self.order_cache.read().await;
        cache.contains(&hash)
    }

    /// Mark an order as being processed
    async fn mark_order_processed(&self, order: &OrderEntity) {
        let hash = order.order_hash.clone();
        let mut cache = self.order_cache.write().await;
        cache.put(hash, ());
    }

    // ----------------
    // | Polling Loop |
    // ----------------

    /// Spawn a polling loop for the UniswapX API
    pub fn spawn_polling_loop(&self) {
        let self_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = self_clone.polling_loop().await {
                error!("Polling loop error: {e}");
                error!("Critical error in polling loop, shutting down process");
                std::process::exit(1);
            }
        });
    }

    /// The inner polling loop
    async fn polling_loop(&self) -> SolverResult<()> {
        loop {
            tokio::time::sleep(POLLING_INTERVAL).await;
            if let Err(e) = self.poll_orders().await {
                error!("Error polling for orders: {e}");
                continue;
            }
        }
    }

    /// Poll the UniswapX API for new orders
    async fn poll_orders(&self) -> SolverResult<()> {
        // Fetch open orders from the API
        let orders = self.fetch_open_orders().await?;

        // Spawn a task to solve each new order
        for order in orders {
            let self_clone = self.clone();
            self.mark_order_processed(&order).await;
            tokio::spawn(async move {
                if let Err(e) = self_clone.solve_order(order).await {
                    error!("Error solving order: {e}");
                }
            });
        }

        Ok(())
    }
}
