//! DB queries for the auth server

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::{
    models::{ApiKey, NewApiKey},
    schema::api_keys,
};

use super::{AuthServerError, Server};

impl Server {
    // --- Getters --- //

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
}
