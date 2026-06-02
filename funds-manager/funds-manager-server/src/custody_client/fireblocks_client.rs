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
    custody_client::{
        fireblocks_rate_limiter::{global_limiter, FireblocksLimiter, FireblocksUserClass, Is429},
        fireblocks_retry_after::RetryAfterCapture,
    },
    error::FundsManagerError,
};

/// TTL for cached vault-balance responses. A cache miss does a live
/// `get_vault_account` Fireblocks call; at 3s, the gardener's parallel sweep
/// across many vaults drove `get_vault_account` calls hard enough to saturate
/// the Fireblocks rate limiter (the 2026-05-29 get-vault-balances 429s). 30s
/// cuts that call rate ~10x with negligible staleness — the gardener syncs on
/// a minutes cadence and balances don't move meaningfully in 30s.
const VAULT_BALANCES_CACHE_TTL: Duration = Duration::from_secs(30);

/// A single Fireblocks API user: its SDK client plus the process-global
/// limiter for that user's per-user rate-limit budget. Fireblocks sets rate
/// limits at the API-user level, so each user (signing / polling / read) has
/// an independent budget.
#[derive(Clone)]
struct FireblocksUser {
    /// The Fireblocks API client authenticated as this user.
    sdk: Client,
    /// Process-global limiter for this user (shared across every chain's
    /// `FireblocksClient` in this process — the budget is per-user, not
    /// per-client).
    limiter: Arc<FireblocksLimiter>,
}

/// A client for interacting with the Fireblocks API. Wraps three API users
/// (signing / polling / read) so reads and tx-status polls don't consume the
/// latency-critical signing user's rate-limit budget (Tip #2). The three users
/// share one keypair today (registered with the same CSR); the split is by
/// API-key UUID, which is what the rate limit is keyed on.
#[derive(Clone)]
pub struct FireblocksClient {
    /// Signing user — POST /v1/transactions (the latency-critical path).
    signing: FireblocksUser,
    /// Polling user — GET transaction status reads.
    polling: FireblocksUser,
    /// Read user — vault / asset / wallet info reads.
    read: FireblocksUser,
    /// Cached metadata from the Fireblocks API, shared across all three users.
    metadata: Arc<RwLock<FireblocksMetadata>>,
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
    /// A mapping from a whitelisted-external-wallet asset address
    /// (lower-cased) to its Fireblocks wallet ID. Populated lazily on the
    /// first `get_external_wallets` call and reused indefinitely — the
    /// whitelist is admin-managed and changes rarely; staleness is
    /// resolved by funds-manager restart.
    pub external_wallet_ids: HashMap<String, String>,
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
            external_wallet_ids: HashMap::new(),
        }
    }
}

impl FireblocksUser {
    /// Build an SDK client for `api_key` / `api_secret`, wired to the
    /// process-global limiter for `class` (including the Retry-After capture
    /// middleware that feeds that limiter's 429 cooldown).
    fn new(
        api_key: &str,
        api_secret: &[u8],
        class: FireblocksUserClass,
    ) -> Result<Self, FundsManagerError> {
        let limiter = global_limiter(class);
        // Install the Retry-After capture middleware so 429 cooldowns honor
        // server-directed backoff; the middleware writes into the same
        // `RetryAfterStore` that this user's limiter `on_429` consumes.
        let retry_after_capture = Arc::new(RetryAfterCapture::new(limiter.retry_after_store()))
            as Arc<dyn reqwest_middleware::Middleware>;
        let sdk = ClientBuilder::new(api_key, api_secret)
            .with_middleware(retry_after_capture)
            .build()
            .map_err(FundsManagerError::fireblocks)?;
        Ok(Self { sdk, limiter })
    }

    /// Run an SDK call as this user, under its limiter. Acquires a token,
    /// invokes the closure, and reports 429/success back so the per-user
    /// consecutive-429 backoff state stays correct.
    async fn rate_limited<'s, T, E, Fut>(
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
}

impl FireblocksClient {
    /// Construct a new Fireblocks client with three API users. The three share
    /// one private key (`fireblocks_api_secret`) but distinct API-key UUIDs;
    /// the rate-limit budget is per-UUID, so routing reads/polls to their own
    /// users keeps the signing user's budget free.
    pub fn new(
        signing_api_key: &str,
        polling_api_key: &str,
        read_api_key: &str,
        fireblocks_api_secret: &str,
    ) -> Result<Self, FundsManagerError> {
        let secret = fireblocks_api_secret.as_bytes().to_vec();
        Ok(FireblocksClient {
            signing: FireblocksUser::new(signing_api_key, &secret, FireblocksUserClass::Signing)?,
            polling: FireblocksUser::new(polling_api_key, &secret, FireblocksUserClass::Polling)?,
            read: FireblocksUser::new(read_api_key, &secret, FireblocksUserClass::Read)?,
            metadata: Arc::new(RwLock::new(FireblocksMetadata::new())),
        })
    }

    // -----------------
    // | Rate limiting |
    // -----------------

    /// Run a Fireblocks SDK call as the SIGNING user (POST /v1/transactions,
    /// the latency-critical path). Use [`rate_limited_read`](Self::rate_limited_read)
    /// / [`rate_limited_poll`](Self::rate_limited_poll) for non-signing calls
    /// so reads and tx-status polls don't consume the signing user's budget.
    pub async fn rate_limited<'s, T, E, Fut>(
        &'s self,
        f: impl FnOnce(&'s Client) -> Fut,
    ) -> Result<T, E>
    where
        Fut: Future<Output = Result<T, E>> + 's,
        E: Is429,
    {
        self.signing.rate_limited(f).await
    }

    /// Run a Fireblocks SDK call as the READ user (vault / asset / wallet info
    /// reads).
    pub async fn rate_limited_read<'s, T, E, Fut>(
        &'s self,
        f: impl FnOnce(&'s Client) -> Fut,
    ) -> Result<T, E>
    where
        Fut: Future<Output = Result<T, E>> + 's,
        E: Is429,
    {
        self.read.rate_limited(f).await
    }

    /// Run a Fireblocks SDK call as the POLLING user (GET transaction status).
    pub async fn rate_limited_poll<'s, T, E, Fut>(
        &'s self,
        f: impl FnOnce(&'s Client) -> Fut,
    ) -> Result<T, E>
    where
        Fut: Future<Output = Result<T, E>> + 's,
        E: Is429,
    {
        self.polling.rate_limited(f).await
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
    pub async fn read_cached_vault_balances(&self, vault_name: &str) -> Option<Vec<TokenBalance>> {
        let metadata = self.metadata.read().await;
        let (cached_at, balances) = metadata.vault_balances.get(vault_name)?;
        if cached_at.elapsed() < VAULT_BALANCES_CACHE_TTL {
            Some(balances.clone())
        } else {
            None
        }
    }

    /// Read a cached whitelisted-external-wallet ID by its lower-cased asset
    /// address.
    pub async fn read_cached_external_wallet_id(&self, address: &str) -> Option<String> {
        self.metadata.read().await.external_wallet_ids.get(&address.to_lowercase()).cloned()
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

    /// Replace the cached whitelisted-external-wallet map. Called after a
    /// fresh `get_external_wallets` fetch so the cache reflects exactly the
    /// current Fireblocks whitelist (entries removed upstream stop being
    /// served from cache).
    pub async fn cache_external_wallet_ids(&self, ids: HashMap<String, String>) {
        self.metadata.write().await.external_wallet_ids = ids;
    }
}
