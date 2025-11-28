//! DB queries for the auth server

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::{
    error::AuthServerError,
    server::{Server, api_handlers::DEFAULT_RELAYER_FEE},
};

use super::{
    models::{
        ApiKey, AssetDefaultFee, FeeResult, NewApiKey, NewAssetDefaultFee, NewRateLimit,
        NewUserFee, RateLimitMethod, RateLimitResult, UserAssetFeeQueryResult,
    },
    schema::{api_keys, asset_default_fees, user_fees},
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
        if let Some(key) = self.cache.get_api_key(api_key) {
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
        self.cache.cache_api_key(key.clone());
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
        self.cache.cache_api_key(new_key.into());
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
        self.cache.mark_key_expired(key_id);
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

    // -----------------------
    // | External Match Fees |
    // -----------------------

    // --- Per-User Fees --- //

    /// Get the per-asset fee for a given user
    ///
    /// If no override is set, this method returns the asset default fee. If
    /// that value is not set the method returns zero
    pub async fn get_user_fee(&self, user_id: Uuid, asset: String) -> Result<f64, AuthServerError> {
        // Check the cache first
        if let Some(fee) = self.cache.get_user_fee(user_id, asset.clone()) {
            return Ok(fee);
        }

        // Otherwise query the db; first for a user-specific fee, then for an
        // asset-specific fee, and finally return zero if no fee is found
        let mut conn = self.get_db_conn().await?;
        let query = "
            SELECT COALESCE(
                (SELECT fee FROM user_fees WHERE id = $1 AND asset = $2),
                (SELECT fee FROM asset_default_fees WHERE asset = $2),
                $3::float4
            ) as fee
        ";

        let fee_res: FeeResult = diesel::sql_query(query)
            .bind::<diesel::sql_types::Uuid, _>(user_id)
            .bind::<diesel::sql_types::Text, _>(&asset)
            .bind::<diesel::sql_types::Float, _>(DEFAULT_RELAYER_FEE as f32)
            .load::<FeeResult>(&mut conn)
            .await
            .map_err(AuthServerError::db)?
            .pop()
            .ok_or(AuthServerError::db("User fee not found"))?;
        drop(conn); // Drop the connection to release the mutable borrow on `self`

        // Cache the fee and return
        let fee = fee_res.fee as f64;
        self.cache.cache_user_fee(user_id, asset, fee);
        Ok(fee)
    }

    /// Set a user fee override for a given API key and asset
    pub async fn set_user_fee_query(
        &self,
        new_user_fee: NewUserFee,
    ) -> Result<(), AuthServerError> {
        let mut conn = self.get_db_conn().await?;

        // Use ON CONFLICT to either insert or update
        diesel::insert_into(user_fees::table)
            .values(&new_user_fee)
            .on_conflict((user_fees::id, user_fees::asset))
            .do_update()
            .set(user_fees::fee.eq(new_user_fee.fee))
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)?;

        // Cache the user fee
        self.cache.cache_user_fee(new_user_fee.id, new_user_fee.asset, new_user_fee.fee as f64);
        Ok(())
    }

    /// Remove a user fee override for a given API key and asset
    pub async fn remove_user_fee_query(
        &self,
        user_id: Uuid,
        asset: String,
    ) -> Result<(), AuthServerError> {
        // Clear the cache entry
        self.cache.clear_user_fee(user_id, asset.clone());

        // Clear the database entry
        let mut conn = self.get_db_conn().await?;
        let num_deleted = diesel::delete(
            user_fees::table.filter(user_fees::id.eq(user_id)).filter(user_fees::asset.eq(asset)),
        )
        .execute(&mut conn)
        .await
        .map_err(AuthServerError::db)?;

        if num_deleted == 0 {
            return Err(AuthServerError::bad_request("User fee override not found"));
        }

        Ok(())
    }

    /// Get the cartesian product of active users and assets with fee
    /// inheritance
    ///
    /// This joins active API keys with asset default fees and left joins with
    /// user overrides. It also includes assets that only have user overrides
    /// but no default fees.
    pub async fn get_user_asset_fees_with_defaults(
        &self,
    ) -> Result<Vec<UserAssetFeeQueryResult>, AuthServerError> {
        let mut conn = self.get_db_conn().await?;

        // The query to join - includes assets with only user overrides
        // TODO: Optimize this query e.g. by adding indices on asset if the table grows
        let query = "
            SELECT 
                api_keys.id as user_id,
                api_keys.description as user_description,
                assets.asset,
                COALESCE(user_fees.fee, asset_default_fees.fee) as fee,
                CASE WHEN user_fees.fee IS NOT NULL THEN true ELSE false END as is_override
            FROM api_keys
            CROSS JOIN (
                SELECT asset FROM asset_default_fees
                UNION
                SELECT DISTINCT asset FROM user_fees
            ) assets
            LEFT JOIN asset_default_fees ON assets.asset = asset_default_fees.asset
            LEFT JOIN user_fees ON api_keys.id = user_fees.id AND assets.asset = user_fees.asset
            WHERE api_keys.is_active = true
            AND (asset_default_fees.fee IS NOT NULL OR user_fees.fee IS NOT NULL)
            ORDER BY api_keys.id, assets.asset
        ";

        diesel::sql_query(query)
            .load::<UserAssetFeeQueryResult>(&mut conn)
            .await
            .map_err(AuthServerError::db)
    }

    // --- Per-Asset Fees --- //

    /// Set the default fee for a given asset
    pub async fn set_asset_default_fee_query(
        &self,
        new_default_fee: NewAssetDefaultFee,
    ) -> Result<(), AuthServerError> {
        let mut conn = self.get_db_conn().await?;

        // Use ON CONFLICT to either insert or update
        diesel::insert_into(asset_default_fees::table)
            .values(&new_default_fee)
            .on_conflict(asset_default_fees::asset)
            .do_update()
            .set(asset_default_fees::fee.eq(new_default_fee.fee))
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)?;

        // Clear the cache entries for the asset
        self.cache.clear_asset_entries(&new_default_fee.asset);
        Ok(())
    }

    /// Get all asset default fees
    pub async fn get_all_asset_default_fees_query(
        &self,
    ) -> Result<Vec<AssetDefaultFee>, AuthServerError> {
        let mut conn = self.get_db_conn().await?;
        asset_default_fees::table
            .load::<AssetDefaultFee>(&mut conn)
            .await
            .map_err(AuthServerError::db)
    }

    /// Remove the default fee for a given asset
    pub async fn remove_asset_default_fee_query(
        &self,
        asset: String,
    ) -> Result<(), AuthServerError> {
        // Clear the cache entries for the asset
        self.cache.clear_asset_entries(&asset);

        // Remove the database entry
        let mut conn = self.get_db_conn().await?;
        let num_deleted =
            diesel::delete(asset_default_fees::table.filter(asset_default_fees::asset.eq(asset)))
                .execute(&mut conn)
                .await
                .map_err(AuthServerError::db)?;

        if num_deleted == 0 {
            return Err(AuthServerError::bad_request("Asset default fee not found"));
        }

        Ok(())
    }

    // ---------------
    // | Rate Limits |
    // ---------------

    /// Get the rate limit for a given API key and method
    ///
    /// Returns the configured rate limit if it exists, otherwise None
    pub async fn get_rate_limit(
        &self,
        api_key_id: Uuid,
        method: RateLimitMethod,
    ) -> Result<Option<u32>, AuthServerError> {
        // Check the cache first (supports negative caching)
        if let Some(cached) = self.cache.get_rate_limit(api_key_id, method) {
            return Ok(cached);
        }

        // Query the database using raw SQL to handle the enum type
        let mut conn = self.get_db_conn().await?;
        let query = "
            SELECT requests_per_minute
            FROM rate_limits
            WHERE api_key_id = $1
              AND method::text = $2
            LIMIT 1
        ";

        let mut result = diesel::sql_query(query)
            .bind::<diesel::sql_types::Uuid, _>(api_key_id)
            .bind::<diesel::sql_types::Text, _>(method.as_str())
            .load::<RateLimitResult>(&mut conn)
            .await
            .map_err(AuthServerError::db)?;
        drop(conn); // Drop the connection to release the mutable borrow on `self`

        // Cache the rate limit (including None for negative caching)
        let limit = result.pop().map(|r| r.requests_per_minute as u32);
        self.cache.cache_rate_limit(api_key_id, method, limit);

        Ok(limit)
    }

    /// Upsert a rate limit for a given API key and method
    pub async fn set_rate_limit_query(
        &self,
        new_rate_limit: NewRateLimit,
    ) -> Result<(), AuthServerError> {
        let api_key_id = new_rate_limit.api_key_id;
        let method = new_rate_limit.method;

        // Use raw SQL for upsert since diesel doesn't handle custom enum types
        // well with on_conflict
        let mut conn = self.get_db_conn().await?;
        let query = "
            INSERT INTO rate_limits (api_key_id, method, requests_per_minute)
            VALUES ($1, $2::rate_limit_method, $3)
            ON CONFLICT (api_key_id, method)
            DO UPDATE SET requests_per_minute = $3
        ";

        diesel::sql_query(query)
            .bind::<diesel::sql_types::Uuid, _>(api_key_id)
            .bind::<diesel::sql_types::Text, _>(method.as_str())
            .bind::<diesel::sql_types::Integer, _>(new_rate_limit.requests_per_minute as i32)
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)?;
        drop(conn);

        // Update the cache with the new rate limit
        self.cache.cache_rate_limit(api_key_id, method, Some(new_rate_limit.requests_per_minute));
        Ok(())
    }
}
