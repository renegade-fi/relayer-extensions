//! Handles key management requests

use crate::models::NewApiKey;
use auth_server_api::CreateApiKeyRequest;
use bytes::Bytes;
use http::HeaderMap;
use uuid::Uuid;
use warp::{filters::path::FullPath, reject::Rejection, reply::Reply};

use crate::ApiError;

use super::{
    helpers::{aes_encrypt, empty_json_reply},
    Server,
};

impl Server {
    /// Add a new API key to the database
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
}
