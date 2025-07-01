//! Handles key management requests

use crate::{
    http_utils::empty_json_reply,
    server::{db::models::NewApiKey, helpers::aes_encrypt},
};
use auth_server_api::CreateApiKeyRequest;
use bytes::Bytes;
use http::HeaderMap;
use serde_json::json;
use tracing::instrument;
use uuid::Uuid;
use warp::{
    filters::path::FullPath,
    reject::Rejection,
    reply::{Reply, WithStatus},
};

use crate::ApiError;

use super::Server;

impl Server {
    /// Add a new API key to the database
    #[instrument(skip_all)]
    pub async fn add_key(
        &self,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        // Deserialize the request
        let req: CreateApiKeyRequest =
            serde_json::from_slice(&body).map_err(ApiError::bad_request)?;

        // Add the key to the database
        let encrypted_secret = aes_encrypt(&req.secret, &self.encryption_key)?;
        let new_key = NewApiKey::new(req.id, encrypted_secret, req.description);
        self.add_key_query(new_key).await.map_err(ApiError::internal)?;

        Ok(empty_json_reply())
    }

    /// Expire an existing API key
    #[instrument(skip_all)]
    pub async fn expire_key(
        &self,
        key_id: Uuid,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        // Expire the key
        self.expire_key_query(key_id).await?;
        Ok(empty_json_reply())
    }

    /// Whitelist an API key for external match flow rate limiting
    ///
    /// A whitelisted key is not subject to the rate limiting based on rebalance
    /// swap costs reported from the bot server.
    ///
    /// Rather, external match requests authorized by the given key will always
    /// be routed to all matching pools in the relayer.
    #[instrument(skip_all)]
    pub async fn whitelist_api_key(
        &self,
        key_id: Uuid,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;
        let reply = warp::reply::json(&json!({
            "whitelisted": true,
        }));
        Ok(reply)
    }

    /// Remove a whitelist entry for an API key
    ///
    /// See the doc comment for `whitelist_api_key` for more information on
    /// whitelisted keys.
    #[instrument(skip_all)]
    pub async fn remove_whitelist_entry(
        &self,
        key_id: Uuid,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;
        let reply = warp::reply::json(&json!({
            "whitelisted": false,
        }));
        Ok(reply)
    }
}
