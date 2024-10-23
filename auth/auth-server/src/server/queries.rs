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
        let mut conn = self.get_db_conn().await?;
        api_keys::table
            .filter(api_keys::id.eq(api_key))
            .limit(1)
            .load::<ApiKey>(&mut conn)
            .await
            .map_err(AuthServerError::db)
            .map(|res| res[0].clone())
    }

    // --- Setters --- //

    /// Add a new API key to the database
    pub async fn add_key_query(&self, new_key: NewApiKey) -> Result<(), AuthServerError> {
        let mut conn = self.get_db_conn().await?;
        diesel::insert_into(api_keys::table)
            .values(&new_key)
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)
            .map(|_| ())
    }

    /// Expire an existing API key
    pub async fn expire_key_query(&self, key_id: Uuid) -> Result<(), AuthServerError> {
        let mut conn = self.get_db_conn().await?;
        diesel::update(api_keys::table.filter(api_keys::id.eq(key_id)))
            .set(api_keys::is_active.eq(false))
            .execute(&mut conn)
            .await
            .map_err(AuthServerError::db)
            .map(|_| ())
    }
}
