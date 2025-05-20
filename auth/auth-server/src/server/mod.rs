//! Defines the server struct and associated functions
//!
//! The server is a dependency injection container for the authentication server
mod api_auth;
pub(crate) mod api_handlers;
pub(crate) mod caching;
pub(crate) mod db;
pub mod gas_estimation;
pub(crate) mod gas_sponsorship;
pub(crate) mod helpers;
pub(crate) mod rate_limiter;
mod setup;

use std::{sync::Arc, time::Duration};

use crate::bundle_store::BundleStore;
use crate::error::AuthServerError;
use crate::telemetry::quote_comparison::handler::QuoteComparisonHandler;
use aes_gcm::Aes128Gcm;
use alloy::signers::k256::ecdsa::SigningKey;
use alloy_primitives::Address;
use bytes::Bytes;
use caching::ApiKeyCache;
use db::DbPool;
use gas_estimation::gas_cost_sampler::GasCostSampler;
use http::header::CONTENT_LENGTH;
use http::{HeaderMap, Method, Response};
use price_reporter_client::PriceReporterClient;
use rate_limiter::AuthServerRateLimiter;
use redis::aio::ConnectionManager;
use renegade_api::auth::add_expiring_auth_to_headers;
use renegade_common::types::chain::Chain;
use renegade_common::types::hmac::HmacKey;
use reqwest::Client;
use tracing::error;

/// The duration for which the admin authentication is valid
const ADMIN_AUTH_DURATION_MS: u64 = 5_000; // 5 seconds

/// The server struct that holds all the necessary components
#[derive(Clone)]
pub struct Server {
    /// The chain for which the server is configured
    pub chain: Chain,
    /// The database connection pool
    pub db_pool: DbPool,
    /// The Redis client
    pub redis_client: ConnectionManager,
    /// The URL of the relayer
    pub relayer_url: String,
    /// The admin key for the relayer
    pub relayer_admin_key: HmacKey,
    /// The management key for the auth server
    pub management_key: HmacKey,
    /// The encryption key for storing API secrets
    pub encryption_key: Aes128Gcm,
    /// The api key cache
    pub api_key_cache: ApiKeyCache,
    /// The HTTP client
    pub client: Client,
    /// The rate limiter
    pub rate_limiter: AuthServerRateLimiter,
    /// The quote metrics recorder
    pub quote_metrics: Option<Arc<QuoteComparisonHandler>>,
    /// Rate at which to sample metrics (0.0 to 1.0)
    pub metrics_sampling_rate: f64,
    /// The address of the gas sponsor address
    pub gas_sponsor_address: Address,
    /// The auth key for the gas sponsor
    pub gas_sponsor_auth_key: SigningKey,
    /// The price reporter client with WebSocket streaming support
    pub price_reporter_client: Arc<PriceReporterClient>,
    /// The gas cost sampler
    pub gas_cost_sampler: Arc<GasCostSampler>,
    /// The minimum order quote amount for which gas sponsorship is allowed,
    /// in whole units of USDC
    pub min_sponsored_order_quote_amount: f64,
    /// The bundle store
    pub bundle_store: BundleStore,
}

// ----------------
// | Core Helpers |
// ----------------

impl Server {
    /// Send a proxied request to the relayer with admin authentication
    pub(crate) async fn send_admin_request(
        &self,
        method: Method,
        path: &str,
        mut headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response<Bytes>, AuthServerError> {
        // Ensure that the content-length header is set correctly
        // so that the relayer can deserialize the proxied request
        headers.insert(CONTENT_LENGTH, body.len().into());

        // Admin authenticate the request
        self.admin_authenticate(path, &mut headers, &body);

        // Forward the request to the relayer
        let url = format!("{}{}", self.relayer_url, path);
        let req = self.client.request(method, &url).headers(headers).body(body);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers().clone();
                let body = resp.bytes().await.map_err(|e| {
                    AuthServerError::Serde(format!("Failed to read response body: {e}"))
                })?;

                let mut response = warp::http::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;

                Ok(response)
            },
            Err(e) => {
                error!("Error proxying request: {}", e);
                Err(AuthServerError::custom(e))
            },
        }
    }

    /// Admin authenticate a request
    fn admin_authenticate(&self, path: &str, headers: &mut HeaderMap, body: &[u8]) {
        let key = self.relayer_admin_key;
        let expiration = Duration::from_millis(ADMIN_AUTH_DURATION_MS);
        add_expiring_auth_to_headers(path, headers, body, &key, expiration);
    }
}
