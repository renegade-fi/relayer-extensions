//! A wrapper around the Fireblocks SDK, augmenting it with helpful features
//! such as response caching and a process-wide rate limiter (token bucket
//! plus 429-triggered cooldown). All outbound Fireblocks SDK calls in
//! funds-manager should go through [`FireblocksClient::rate_limited`] so
//! that the limiter sees them and stays within the workspace quota.

use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use fireblocks_sdk::{models::AssetOnchainBeta, Client, ClientBuilder};
use funds_manager_api::hot_wallets::TokenBalance;
use tokio::sync::RwLock;

use crate::{
    custody_client::fireblocks_rate_limiter::{global_limiter, FireblocksLimiter, Is429},
    error::FundsManagerError,
};

/// TTL for cached vault-balance responses. Short enough to keep telemetry
/// fresh; long enough to coalesce retry storms (gardener fetch-holdings
/// retries ~1/20s per stuck token).
const VAULT_BALANCES_CACHE_TTL: Duration = Duration::from_secs(3);

/// A client for interacting with the Fireblocks API
#[derive(Clone)]
pub struct FireblocksClient {
    /// The Fireblocks API client
    pub sdk: Client,
    /// Cached metadata from the Fireblocks API
    metadata: Arc<RwLock<FireblocksMetadata>>,
    /// Process-wide rate limiter for Fireblocks calls
    limiter: Arc<FireblocksLimiter>,
}

/// Cached metadata from the Fireblocks API
struct FireblocksMetadata {
    /// A mapping from a vault name to its Fireblocks ID
    pub vault_ids: HashMap<String, String>,
    /// A mapping from an asset's mint to its Fireblocks asset ID
    pub asset_ids: HashMap<String, String>,
    /// A mapping from (vault name, mint) to the deposit address
    pub deposit_addresses: HashMap<(String, String), String>,
    /// A mapping from asset ID to its onchain data
    pub asset_onchain_data: HashMap<String, AssetOnchainBeta>,
    /// A mapping from vault name to a recently-fetched balance snapshot.
    /// Expired entries are filtered on read.
    pub vault_balances: HashMap<String, (Instant, Vec<TokenBalance>)>,
}

impl FireblocksMetadata {
    /// Construct an empty Fireblocks metadata object
    fn new() -> Self {
        Self {
            vault_ids: HashMap::new(),
            asset_ids: HashMap::new(),
            deposit_addresses: HashMap::new(),
            asset_onchain_data: HashMap::new(),
            vault_balances: HashMap::new(),
        }
    }
}

impl FireblocksClient {
    /// Construct a new Fireblocks client. Reuses the process-wide rate
    /// limiter so every `CustodyClient` shares one token bucket — this
    /// matches the actual Fireblocks quota, which is per-workspace.
    pub fn new(
        fireblocks_api_key: &str,
        fireblocks_api_secret: &str,
    ) -> Result<Self, FundsManagerError> {
        let fireblocks_api_secret = fireblocks_api_secret.as_bytes().to_vec();
        let fireblocks_sdk = ClientBuilder::new(fireblocks_api_key, &fireblocks_api_secret)
            .build()
            .map_err(FundsManagerError::fireblocks)?;

        let fireblocks_client = FireblocksClient {
            sdk: fireblocks_sdk,
            metadata: Arc::new(RwLock::new(FireblocksMetadata::new())),
            limiter: global_limiter(),
        };

        Ok(fireblocks_client)
    }

    // -----------------
    // | Rate limiting |
    // -----------------

    /// Run a Fireblocks SDK call under the process-wide rate limiter.
    /// Acquires a token before invoking the closure and reports the
    /// outcome (429 vs. success vs. other-error) back to the limiter so
    /// that consecutive-429 backoff state stays correct.
    ///
    /// All call sites that touch the Fireblocks API should use this
    /// helper. Calls that bypass it consume quota without contributing to
    /// the limiter's view of the workspace load.
    pub async fn rate_limited<'s, T, E, Fut>(
        &'s self,
        f: impl FnOnce(&'s Client) -> Fut,
    ) -> Result<T, E>
    where
        Fut: Future<Output = Result<T, E>> + 's,
        E: Is429,
    {
        self.limiter.acquire().await;
        let result = f(&self.sdk).await;
        match &result {
            Err(e) if e.is_429() => self.limiter.on_429().await,
            Ok(_) => self.limiter.on_success(),
            _ => {},
        }
        result
    }

    // -----------
    // | Getters |
    // -----------

    /// Read a cached vault ID from the metadata
    pub async fn read_cached_vault_id(&self, vault_name: &str) -> Option<String> {
        self.metadata.read().await.vault_ids.get(vault_name).cloned()
    }

    /// Read a cached asset ID from the metadata
    pub async fn read_cached_asset_id(&self, mint: &str) -> Option<String> {
        self.metadata.read().await.asset_ids.get(mint).cloned()
    }

    /// Read a cached deposit address from the metadata
    pub async fn read_cached_deposit_address(
        &self,
        vault_name: String,
        mint: String,
    ) -> Option<String> {
        self.metadata.read().await.deposit_addresses.get(&(vault_name, mint)).cloned()
    }

    /// Read an asset's cached onchain data from the metadata
    pub async fn read_cached_asset_onchain_data(&self, asset_id: &str) -> Option<AssetOnchainBeta> {
        self.metadata.read().await.asset_onchain_data.get(asset_id).cloned()
    }

    /// Read a vault's cached token balances, if still within TTL
    pub async fn read_cached_vault_balances(
        &self,
        vault_name: &str,
    ) -> Option<Vec<TokenBalance>> {
        let metadata = self.metadata.read().await;
        let (cached_at, balances) = metadata.vault_balances.get(vault_name)?;
        if cached_at.elapsed() < VAULT_BALANCES_CACHE_TTL {
            Some(balances.clone())
        } else {
            None
        }
    }

    // -----------
    // | Setters |
    // -----------

    /// Cache a vault ID
    pub async fn cache_vault_id(&self, vault_name: String, vault_id: String) {
        self.metadata.write().await.vault_ids.insert(vault_name, vault_id);
    }

    /// Cache an asset ID
    pub async fn cache_asset_id(&self, mint: String, asset_id: String) {
        self.metadata.write().await.asset_ids.insert(mint, asset_id);
    }

    /// Cache a deposit address
    pub async fn cache_deposit_address(
        &self,
        vault_name: String,
        mint: String,
        deposit_address: String,
    ) {
        self.metadata.write().await.deposit_addresses.insert((vault_name, mint), deposit_address);
    }

    /// Cache an asset's onchain data
    pub async fn cache_asset_onchain_data(
        &self,
        asset_id: String,
        asset_onchain_data: AssetOnchainBeta,
    ) {
        self.metadata.write().await.asset_onchain_data.insert(asset_id, asset_onchain_data);
    }

    /// Cache a vault's token balances with the current timestamp
    pub async fn cache_vault_balances(&self, vault_name: String, balances: Vec<TokenBalance>) {
        self.metadata.write().await.vault_balances.insert(vault_name, (Instant::now(), balances));
    }
}
