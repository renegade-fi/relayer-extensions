//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use auth_server_api::RENEGADE_API_KEY_HEADER;
use bytes::Bytes;
use http::{HeaderMap, Method};
use renegade_api::auth::validate_expiring_auth;
use renegade_common::types::wallet::keychain::HmacKey;
use tracing::error;
use uuid::Uuid;
use warp::{reject::Rejection, reply::Reply};

use crate::{error::AuthServerError, ApiError};

use super::{helpers::aes_decrypt, Server};

/// Handle a proxied request
impl Server {
    /// Handle a request meant to be authenticated and proxied to the relayer
    pub async fn handle_proxy_request(
        &self,
        path: warp::path::FullPath,
        method: Method,
        mut headers: warp::hyper::HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        self.authorize_request(path.as_str(), &mut headers, &body).await?;

        // Forward the request to the relayer
        let url = format!("{}{}", self.relayer_url, path.as_str());
        let req = self.client.request(method, &url).headers(headers).body(body);

        // TODO: Add admin auth here
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers().clone();
                let body = resp.bytes().await.map_err(|e| {
                    warp::reject::custom(ApiError::InternalError(format!(
                        "Failed to read response body: {e}"
                    )))
                })?;

                let mut response = warp::http::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;

                Ok(response)
            },
            Err(e) => {
                error!("Error proxying request: {}", e);
                Err(warp::reject::custom(ApiError::InternalError(e.to_string())))
            },
        }
    }

    /// Authorize a request
    async fn authorize_request(
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
