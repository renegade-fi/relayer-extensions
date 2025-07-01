//! Handles API authentication

use auth_server_api::RENEGADE_API_KEY_HEADER;
use http::HeaderMap;
use renegade_api::auth::validate_expiring_auth;
use renegade_common::types::hmac::HmacKey;
use tracing::{info, instrument};
use uuid::Uuid;
use warp::filters::path::FullPath;

use crate::{error::AuthServerError, http_utils::convert_headers, ApiError};

use super::{helpers::aes_decrypt, Server};

impl Server {
    /// Authorize a management request
    pub fn authorize_management_request(
        &self,
        path: &FullPath,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<(), ApiError> {
        let auth_headers = convert_headers(headers);
        validate_expiring_auth(path.as_str(), &auth_headers, body, &self.management_key)
            .map_err(|_| ApiError::Unauthorized)
    }

    /// Authorize a request
    ///
    /// Returns the description for the API key, i.e. a human readable name for
    /// the entity that is making the request
    #[instrument(skip_all)]
    pub(crate) async fn authorize_request(
        &self,
        path: &str,
        query_str: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<String, ApiError> {
        let auth_path =
            if query_str.is_empty() { path } else { &format!("{}?{}", path, query_str) };

        // Check API auth
        let api_key = headers
            .get(RENEGADE_API_KEY_HEADER)
            .and_then(|h| h.to_str().ok().map(String::from)) // Convert to String
            .and_then(|s| Uuid::parse_str(&s).ok()) // Use &s to parse
            .ok_or(AuthServerError::unauthorized("Invalid or missing Renegade API key"))?;

        let key_description = self.check_api_key_auth(api_key, auth_path, headers, body).await?;
        info!("Authorized request for entity: {key_description}");
        Ok(key_description)
    }

    /// Check that a request is authorized with a given API key and an HMAC of
    /// the request using the API secret
    ///
    /// Returns the description for the API key, i.e. a human readable name for
    /// the entity that is making the request
    async fn check_api_key_auth(
        &self,
        api_key: Uuid,
        path: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<String, AuthServerError> {
        let (api_secret, description) = self.get_api_secret(api_key).await?;
        let key = HmacKey::from_base64_string(&api_secret).map_err(AuthServerError::serde)?;

        let auth_headers = convert_headers(headers);
        validate_expiring_auth(path, &auth_headers, body, &key)
            .map_err(AuthServerError::unauthorized)?;
        Ok(description)
    }

    /// Get the API secret for a given API key
    ///
    /// Also returns the description for the API key, i.e. a human readable name
    /// for the entity that is making the request
    async fn get_api_secret(&self, api_key: Uuid) -> Result<(String, String), AuthServerError> {
        // Fetch the API key entry then decrypt the API secret
        let entry = self.get_api_key_entry(api_key).await?;
        let decrypted = aes_decrypt(&entry.encrypted_key, &self.encryption_key)?;
        if !entry.is_active {
            return Err(AuthServerError::ApiKeyInactive);
        }

        Ok((decrypted, entry.description))
    }
}
