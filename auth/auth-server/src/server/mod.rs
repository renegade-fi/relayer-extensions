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
pub(crate) mod http;
pub(crate) mod rate_limiter;
mod setup;
pub mod worker;

use std::str::FromStr;
use std::{sync::Arc, time::Duration};

use crate::bundle_store::BundleStore;
use crate::error::AuthServerError;
use crate::http_utils::request_response::convert_headers;
use crate::server::caching::ServerCache;
use ::http::header::CONTENT_LENGTH;
use ::http::{HeaderMap, HeaderName, HeaderValue, Method, Response};
use aes_gcm::Aes128Gcm;
use alloy::signers::k256::ecdsa::SigningKey;
use alloy_primitives::Address;
use base64::engine::{Engine, general_purpose as b64_general_purpose};
use bytes::Bytes;
use db::DbPool;
use gas_estimation::gas_cost_sampler::GasCostSampler;
use price_reporter_client::PriceReporterClient;
use rate_limiter::AuthServerRateLimiter;
use redis::aio::ConnectionManager;
use renegade_api::auth::create_request_signature;
use renegade_api::{RENEGADE_AUTH_HEADER_NAME, RENEGADE_SIG_EXPIRATION_HEADER_NAME};
use renegade_common::types::chain::Chain;
use renegade_common::types::hmac::HmacKey;
use renegade_util::get_current_time_millis;
use renegade_util::telemetry::propagation::trace_context;
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
    /// The server's data cache
    pub cache: ServerCache,
    /// The HTTP client
    pub client: Client,
    /// The rate limiter
    pub rate_limiter: AuthServerRateLimiter,
    /// Rate at which to sample metrics (0.0 to 1.0)
    pub metrics_sampling_rate: f64,
    /// The address of the gas sponsor address
    pub gas_sponsor_address: Address,
    /// The address of the malleable match connector contract
    pub malleable_match_connector_address: Address,
    /// The auth key for the gas sponsor
    pub gas_sponsor_auth_key: SigningKey,
    /// The price reporter client with WebSocket streaming support
    pub price_reporter_client: PriceReporterClient,
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

        // Inject OpenTelemetry context propagation headers so the relayer
        // can join the same distributed trace (e.g. Datadog/APM)
        self.add_trace_context_headers(&mut headers);

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
    ///
    /// We copy the inner auth logic here because we need to convert the headers
    /// to work with the relayer's `http` crate version
    fn admin_authenticate(&self, path: &str, headers: &mut HeaderMap, body: &[u8]) {
        let key = self.relayer_admin_key;
        let expiration = Duration::from_millis(ADMIN_AUTH_DURATION_MS);

        // Add a timestamp
        let expiration_ts = get_current_time_millis() + expiration.as_millis() as u64;
        headers.insert(RENEGADE_SIG_EXPIRATION_HEADER_NAME, expiration_ts.into());

        // Add the signature
        let converted_headers = convert_headers(headers);
        let sig = create_request_signature(path, &converted_headers, body, &key);
        let b64_sig = b64_general_purpose::STANDARD_NO_PAD.encode(sig);
        let sig_header = HeaderValue::from_str(&b64_sig).expect("b64 encoding should not fail");
        headers.insert(RENEGADE_AUTH_HEADER_NAME, sig_header);
    }

    /// Add the trace context headers to the request
    fn add_trace_context_headers(&self, headers: &mut HeaderMap) {
        for (key, value) in trace_context() {
            let maybe_name = HeaderName::from_str(&key);
            let maybe_val = HeaderValue::from_str(&value);
            if let (Ok(name), Ok(val)) = (maybe_name, maybe_val) {
                headers.append(name, val);
            }
        }
    }
}
