//! Handles API authentication

use auth_server_api::RENEGADE_API_KEY_HEADER;
use http::HeaderMap;
use renegade_api::auth::validate_expiring_auth;
use renegade_common::types::wallet::keychain::HmacKey;
use uuid::Uuid;
use warp::filters::path::FullPath;

use crate::{error::AuthServerError, ApiError};

use super::{helpers::aes_decrypt, Server};

impl Server {
    /// Authorize a management request
    pub fn authorize_management_request(
        &self,
        path: &FullPath,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<(), ApiError> {
        validate_expiring_auth(path.as_str(), headers, body, &self.management_key)
            .map_err(|_| ApiError::Unauthorized)
    }

    /// Authorize a request
    pub(crate) async fn authorize_request(
        &self,
        path: &str,
        headers: &mut HeaderMap,
        body: &[u8],
    ) -> Result<(), ApiError> {
        // Check API auth
        let api_key = headers
            .remove(RENEGADE_API_KEY_HEADER)
            .and_then(|h| h.to_str().ok().map(String::from)) // Convert to String
            .and_then(|s| Uuid::parse_str(&s).ok()) // Use &s to parse
            .ok_or(AuthServerError::unauthorized("Invalid or missing Renegade API key"))?;

        self.check_api_key_auth(api_key, path, headers, body).await?;
        Ok(())
    }

    /// Check that a request is authorized with a given API key and an HMAC of
    /// the request using the API secret
    async fn check_api_key_auth(
        &self,
        api_key: Uuid,
        path: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<(), AuthServerError> {
        let api_secret = self.get_api_secret(api_key).await?;
        let key = HmacKey::from_base64_string(&api_secret).map_err(AuthServerError::serde)?;

        validate_expiring_auth(path, headers, body, &key).map_err(AuthServerError::unauthorized)
    }

    /// Get the API secret for a given API key
    async fn get_api_secret(&self, api_key: Uuid) -> Result<String, AuthServerError> {
        // Fetch the API key entry then decrypt the API secret
        let entry = self.get_api_key_entry(api_key).await?;
        let decrypted = aes_decrypt(&entry.encrypted_key, &self.encryption_key)?;
        if !entry.is_active {
            return Err(AuthServerError::ApiKeyInactive);
        }

        Ok(decrypted)
    }
}