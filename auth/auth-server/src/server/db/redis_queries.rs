//! Helpers for interacting with Redis

use auth_server_api::GasSponsorshipInfo;
use redis::{AsyncCommands, JsonAsyncCommands};
use uuid::Uuid;

use crate::{error::AuthServerError, server::Server};

// -------------
// | Constants |
// -------------

/// The root path for a JSON object in Redis
const JSON_ROOT_PATH: &str = "$";

/// The maximum age of a quote in milliseconds.
/// Equivalently, this is the TTL for the quote's gas sponsorship entry in
/// Redis.
const MAX_QUOTE_AGE_MS: i64 = 10_000; // 10 seconds

impl Server {
    // -----------
    // | Setters |
    // -----------

    /// Write the given gas sponsorship info to Redis
    pub async fn write_gas_sponsorship_info_to_redis(
        &self,
        key: Uuid,
        info: &GasSponsorshipInfo,
    ) -> Result<(), AuthServerError> {
        let mut client = self.redis_client.clone();
        client
            .json_set::<_, _, _, ()>(key, JSON_ROOT_PATH, info)
            .await
            .map_err(AuthServerError::redis)?;

        client.pexpire(key, MAX_QUOTE_AGE_MS).await.map_err(AuthServerError::redis)
    }

    // -----------
    // | Getters |
    // -----------

    /// Read the gas sponsorship info for the given key from Redis,
    /// returning `None` if no info is found
    pub async fn read_gas_sponsorship_info_from_redis(
        &self,
        key: Uuid,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError> {
        let mut client = self.redis_client.clone();
        let info_str: Option<String> =
            client.json_get(key, JSON_ROOT_PATH).await.map_err(AuthServerError::redis)?;

        if info_str.is_none() {
            return Ok(None);
        }

        // We have to deserialize to `Vec<GasSponsorshipInfo>` as per https://docs.rs/redis/latest/redis/trait.JsonAsyncCommands.html#method.json_get?
        let mut info: Vec<GasSponsorshipInfo> =
            serde_json::from_str(&info_str.unwrap()).map_err(AuthServerError::serde)?;

        Ok(Some(info.swap_remove(0)))
    }
}
