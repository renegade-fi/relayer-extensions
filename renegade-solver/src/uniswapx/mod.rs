//! UniswapX API client and handlers

use std::{str::FromStr, sync::Arc, time::Duration};

use crate::{cli::Cli, error::SolverResult, uniswapx::uniswap_api::types::OrderEntity};
use alloy::primitives::Address;
use bimap::BiMap;
use executor_client::ExecutorClient;
use lru::LruCache;
use renegade_sdk::ExternalMatchClient;
use reqwest::Client as ReqwestClient;
use tokio::sync::RwLock;
use tracing::error;

mod abis;
pub mod executor_client;
mod helpers;
mod renegade_api;
mod solve;
mod uniswap_api;

/// The interval at which to poll for new orders
const POLLING_INTERVAL: Duration = Duration::from_secs(1);

/// The maximum number of orders to cache
///
/// Orders are typically only duplicated for a short window while the auction
/// commences, so a small cache is sufficient.
const ORDER_CACHE_SIZE: usize = 100;
/// The symbol for native ETH
const NATIVE_ETH_SYMBOL: &str = "ETH";
/// The symbol for USDC
const USDC_SYMBOL: &str = "USDC";

/// A shared read-only bimap of supported tokens
///
/// Maps bidirectionally between address and symbol
type SupportedTokens = Arc<BiMap<Address, String>>;
/// A shared read-only LRU cache of order hashes we've already tried to handle
type OrderCache = Arc<RwLock<LruCache<String, ()>>>;

// ------------------
// | Helper Methods |
// ------------------

/// Create a new order cache
fn new_order_cache() -> OrderCache {
    let cache_size = std::num::NonZeroUsize::new(ORDER_CACHE_SIZE).unwrap();
    Arc::new(RwLock::new(LruCache::new(cache_size)))
}

// ----------
// | Solver |
// ----------

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
    /// The executor client for submitting solutions
    executor_client: Arc<ExecutorClient>,
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
    pub async fn new(cli: Cli, executor_client: ExecutorClient) -> SolverResult<Self> {
        let Cli { uniswapx_url: base_url, renegade_api_key, renegade_api_secret, .. } = cli;

        // TODO: Add support for other chains
        let renegade_client =
            ExternalMatchClient::new_base_mainnet_client(&renegade_api_key, &renegade_api_secret)?;
        let supported_tokens = Self::load_supported_tokens(&renegade_client).await?;

        Ok(Self {
            base_url,
            http_client: ReqwestClient::new(),
            renegade_client,
            executor_client: Arc::new(executor_client),
            supported_tokens,
            order_cache: new_order_cache(),
        })
    }

    /// Load the known tokens from the database
    async fn load_supported_tokens(client: &ExternalMatchClient) -> SolverResult<SupportedTokens> {
        // Build a bimap and insert the zero address in place of native ETH
        let mut map = BiMap::new();
        map.insert(Address::ZERO, NATIVE_ETH_SYMBOL.to_string());

        // Insert all tokens from the external match API
        let resp = client.get_supported_tokens().await?;
        for token in resp.tokens {
            let addr = Address::from_str(&token.address).expect("Invalid supported token address");
            map.insert(addr, token.symbol);
        }

        Ok(Arc::new(map))
    }

    // -----------
    // | Helpers |
    // -----------

    /// Check if a token is supported
    fn is_token_supported(&self, token: Address) -> bool {
        self.supported_tokens.contains_left(&token)
    }

    /// Returns whether the given token is USDC
    pub(crate) fn is_usdc(&self, token: Address) -> bool {
        token == self.get_usdc_address()
    }

    /// Get the USDC token address
    fn get_usdc_address(&self) -> Address {
        self.get_token_address(USDC_SYMBOL).expect("USDC is not supported")
    }

    /// Get the address for a token symbol
    fn get_token_address(&self, symbol: &str) -> Option<Address> {
        self.supported_tokens.get_by_right(symbol).copied()
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
