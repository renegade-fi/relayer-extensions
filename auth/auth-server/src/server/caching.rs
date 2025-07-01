//! Caching helpers for the auth server

use std::sync::Arc;

use cached::{Cached, UnboundCache};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::Server;

use super::db::models::ApiKey;

/// The API key cache type
pub type ApiKeyCache = Arc<RwLock<UnboundCache<Uuid, ApiKey>>>;

impl Server {
    /// Check the cache for an API key
    pub async fn get_cached_api_key(&self, id: Uuid) -> Option<ApiKey> {
        let cache = self.api_key_cache.read().await;
        cache.get_store().get(&id).cloned()
    }

    /// Cache an API key
    pub async fn cache_api_key(&self, api_key: ApiKey) {
        let mut cache = self.api_key_cache.write().await;
        cache.cache_set(api_key.id, api_key);
    }

    /// Mark a cached API key as expired
    pub async fn mark_cached_key_expired(&self, id: Uuid) {
        let mut cache = self.api_key_cache.write().await;
        if let Some(key) = cache.cache_get_mut(&id) {
            key.is_active = false;
        }
    }

    /// Clear the cache entry for a given API key
    ///
    /// We use this as a simpler way to invalidate a key so that it is
    /// re-hydrated from the DB
    pub async fn clear_cached_key(&self, id: Uuid) {
        let mut cache = self.api_key_cache.write().await;
        cache.cache_remove(&id);
    }
}
