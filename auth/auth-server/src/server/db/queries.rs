//! DB queries for the auth server

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::{error::AuthServerError, server::Server};

use super::{
    models::{ApiKey, NewApiKey},
    schema::api_keys,
};

/// Error returned when a key is not found in the database
const ERR_NO_KEY: &str = "API key not found";

impl Server {
    // --- Getters --- //

    /// Get all API keys from the database
    pub async fn get_all_api_keys(&self) -> Result<Vec<ApiKey>, AuthServerError> {
        let mut conn = self.get_db_conn().await?;
        api_keys::table.load::<ApiKey>(&mut conn).await.map_err(AuthServerError::db)
    }

    /// Get the API key entry for a given key
    pub async fn get_api_key_entry(&self, api_key: Uuid) -> Result<ApiKey, AuthServerError> {
        // Check the cache first
        if let Some(key) = self.get_cached_api_secret(api_key).await {
            return Ok(key);
        }

        // Fetch the key from the database
        let mut conn = self.get_db_conn().await?;
        let result = api_keys::table
            .filter(api_keys::id.eq(api_key))
            .limit(1)
            .load::<ApiKey>(&mut conn)
            .await
            .map_err(AuthServerError::db)?;
        drop(conn); // Drop the connection to release the mutable borrow on `self`

        let key = if result.is_empty() {
            return Err(AuthServerError::unauthorized("API key not found"));
        } else {
            result[0].clone()
        };

        // Cache the key and return
        self.cache_api_key(key.clone()).await;
        if !key.is_active {
            return Err(AuthServerError::ApiKeyInactive);
        }

        Ok(key)
    }

    // --- Setters --- //

    /// Add a new API key to the database
    pub async fn add_key_query(&self, new_key: NewApiKey) -> Result<(), AuthServerError> {
        // Write to the database
        let mut conn = self.get_db_conn().await?;
        diesel::insert_into(api_keys::table)
            .values(&new_key)
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)?;
        drop(conn); // Drop the connection to release the mutable borrow on `self`

        // Cache the key
        self.cache_api_key(new_key.into()).await;
        Ok(())
    }

    /// Expire an existing API key
    pub async fn expire_key_query(&self, key_id: Uuid) -> Result<(), AuthServerError> {
        // Update the database
        let mut conn = self.get_db_conn().await?;
        diesel::update(api_keys::table.filter(api_keys::id.eq(key_id)))
            .set(api_keys::is_active.eq(false))
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)?;
        drop(conn); // Drop the connection to release the mutable borrow on `self`

        // Remove the key from the cache
        self.mark_cached_key_expired(key_id).await;
        Ok(())
    }

    /// Whitelist an API key for external match flow rate limiting
    pub async fn whitelist_api_key_query(&self, key_id: Uuid) -> Result<(), AuthServerError> {
        let mut conn = self.get_db_conn().await?;
        let num_updates = diesel::update(api_keys::table.filter(api_keys::id.eq(key_id)))
            .set(api_keys::rate_limit_whitelisted.eq(true))
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)?;

        // Check that an update was made
        if num_updates == 0 {
            return Err(AuthServerError::bad_request(ERR_NO_KEY));
        }
        Ok(())
    }

    /// Remove a whitelist entry for an API key
    pub async fn remove_whitelist_entry_query(&self, key_id: Uuid) -> Result<(), AuthServerError> {
        let mut conn = self.get_db_conn().await?;
        let num_updates = diesel::update(api_keys::table.filter(api_keys::id.eq(key_id)))
            .set(api_keys::rate_limit_whitelisted.eq(false))
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)?;

        // Check that an update was made
        if num_updates == 0 {
            return Err(AuthServerError::bad_request(ERR_NO_KEY));
        }
        Ok(())
    }
}
