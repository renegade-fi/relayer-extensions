//! Handles key management requests

use crate::models::NewApiKey;
use auth_server_api::CreateApiKeyRequest;
use uuid::Uuid;
use warp::{reject::Rejection, reply::Reply};

use crate::ApiError;

use super::{
    helpers::{aes_encrypt, empty_json_reply},
    Server,
};

impl Server {
    /// Add a new API key to the database
    pub async fn add_key(&self, req: CreateApiKeyRequest) -> Result<impl Reply, Rejection> {
        let encrypted_secret = aes_encrypt(&req.secret, &self.encryption_key)?;
        let new_key = NewApiKey::new(req.id, encrypted_secret, req.description);
        self.add_key_query(new_key)
            .await
            .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

        Ok(empty_json_reply())
    }

    /// Expire an existing API key
    pub async fn expire_key(&self, key_id: Uuid) -> Result<impl Reply, Rejection> {
        self.expire_key_query(key_id)
            .await
            .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

        Ok(empty_json_reply())
    }
}
