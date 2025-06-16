//! A wrapper around the Fireblocks SDK, augmenting it with helpful features
//! such as response caching

use std::{collections::HashMap, sync::Arc};

use fireblocks_sdk::{models::AssetOnchainBeta, Client, ClientBuilder};
use tokio::sync::RwLock;

use crate::error::FundsManagerError;

/// A client for interacting with the Fireblocks API
#[derive(Clone)]
pub struct FireblocksClient {
    /// The Fireblocks API client
    pub sdk: Client,
    /// Cached metadata from the Fireblocks API
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
}

impl FireblocksMetadata {
    /// Construct an empty Fireblocks metadata object
    fn new() -> Self {
        Self {
            vault_ids: HashMap::new(),
            asset_ids: HashMap::new(),
            deposit_addresses: HashMap::new(),
            asset_onchain_data: HashMap::new(),
        }
    }
}

impl FireblocksClient {
    /// Construct a new Fireblocks client
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
        };

        Ok(fireblocks_client)
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
}
