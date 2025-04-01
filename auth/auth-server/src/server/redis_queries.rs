//! Helpers for interacting with Redis

use auth_server_api::GasSponsorshipInfo;
use redis::JsonAsyncCommands;
use uuid::Uuid;

use crate::error::AuthServerError;

use super::Server;

// -------------
// | Constants |
// -------------

/// The root path for a JSON object in Redis
const JSON_ROOT_PATH: &str = "$";

impl Server {
    // -----------
    // | Setters |
    // -----------

    /// Write the given gas sponsorship info to Redis
    pub async fn write_gas_sponsorship_info(
        &self,
        key: Uuid,
        info: &GasSponsorshipInfo,
    ) -> Result<(), AuthServerError> {
        let mut client = self.redis_client.clone();
        client.json_set(key, JSON_ROOT_PATH, info).await.map_err(AuthServerError::redis)
    }

    // -----------
    // | Getters |
    // -----------

    /// Read the gas sponsorship info for the given key from Redis,
    /// returning `None` if no info is found
    pub async fn read_gas_sponsorship_info(
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
