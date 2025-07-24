//! Caching helpers for the auth server

use dashmap::DashMap;
use uuid::Uuid;

use super::db::models::ApiKey;

/// The API key cache type
pub type ApiKeyCache = DashMap<Uuid, ApiKey>;
/// The user fee cache type
///
/// Maps from (user_id, asset) to the fee rate for that asset
pub type UserFeeCache = DashMap<(Uuid, String), f64>;

/// The Server's data cache
#[derive(Clone)]
pub struct ServerCache {
    /// The API key cache
    pub api_key_cache: ApiKeyCache,
    /// The user fee cache
    pub user_fee_cache: UserFeeCache,
}

impl ServerCache {
    /// Constructor
    pub fn new() -> Self {
        Self { api_key_cache: DashMap::new(), user_fee_cache: DashMap::new() }
    }

    // --- Api Key Cache --- //

    /// Check the cache for an API key
    pub fn get_api_key(&self, id: Uuid) -> Option<ApiKey> {
        self.api_key_cache.get(&id).map(|ptr| ptr.value().clone())
    }

    /// Cache an API key
    pub fn cache_api_key(&self, api_key: ApiKey) {
        self.api_key_cache.insert(api_key.id, api_key);
    }

    /// Mark a cached API key as expired
    pub fn mark_key_expired(&self, id: Uuid) {
        if let Some(mut key) = self.api_key_cache.get_mut(&id) {
            key.value_mut().is_active = false;
        }
    }

    /// Clear the cache entry for a given API key
    pub fn clear_key(&self, id: Uuid) {
        self.api_key_cache.remove(&id);
    }

    // --- User Fee Cache --- //

    /// Check the cache for a user fee
    pub fn get_user_fee(&self, user_id: Uuid, asset: String) -> Option<f64> {
        self.user_fee_cache.get(&(user_id, asset)).map(|ptr| *ptr.value())
    }

    /// Cache a user fee
    pub fn cache_user_fee(&self, user_id: Uuid, asset: String, fee: f64) {
        self.user_fee_cache.insert((user_id, asset), fee);
    }

    /// Clear the cache entry for a user fee
    pub fn clear_user_fee(&self, user_id: Uuid, asset: String) {
        self.user_fee_cache.remove(&(user_id, asset));
    }

    /// Clear the cache entries for a given asset
    pub fn clear_asset_entries(&self, asset: &str) {
        self.user_fee_cache.retain(|(_, asset_name), _| asset_name != asset);
    }
}
