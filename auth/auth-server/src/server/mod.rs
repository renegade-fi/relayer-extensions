//! Defines the server struct and associated functions
//!
//! The server is a dependency injection container for the authentication server
mod api_auth;
pub mod gas_estimation;
pub(crate) mod handle_external_match;
mod handle_key_management;
pub(crate) mod helpers;
mod order_book;
pub mod price_reporter_client;
mod queries;
pub(crate) mod rate_limiter;
mod redis_queries;

use std::{iter, sync::Arc, time::Duration};

use crate::chain_events::listener::{OnChainEventListener, OnChainEventListenerConfig};
use crate::helpers::create_arbitrum_client;
use crate::server::price_reporter_client::PriceReporterClient;
use crate::store::BundleStore;
use crate::{
    error::AuthServerError,
    models::ApiKey,
    telemetry::{quote_comparison::handler::QuoteComparisonHandler, sources::QuoteSource},
    ApiError, Cli,
};
use aes_gcm::{Aes128Gcm, KeyInit};
use base64::{engine::general_purpose, Engine};
use bb8::{Pool, PooledConnection};
use bytes::Bytes;
use cached::{Cached, UnboundCache};
use diesel::ConnectionError;
use diesel_async::{
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig},
    AsyncPgConnection,
};
use ethers::{abi::Address, core::k256::ecdsa::SigningKey, utils::hex};
use gas_estimation::gas_cost_sampler::GasCostSampler;
use http::header::CONTENT_LENGTH;
use http::{HeaderMap, Method, Response};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use rand::Rng;
use rate_limiter::AuthServerRateLimiter;
use redis::aio::ConnectionManager;
use renegade_api::auth::add_expiring_auth_to_headers;
use renegade_arbitrum_client::client::ArbitrumClient;
use renegade_common::types::{
    hmac::HmacKey,
    token::{get_all_tokens, Token},
};
use renegade_config::setup_token_remaps;
use renegade_constants::NATIVE_ASSET_ADDRESS;
use renegade_system_clock::SystemClock;
use renegade_util::{
    on_chain::{set_external_match_fee, PROTOCOL_FEE},
    telemetry::configure_telemetry,
};
use reqwest::Client;
use tokio::sync::RwLock;
use tracing::{error, warn};
use uuid::Uuid;

/// The duration for which the admin authentication is valid
const ADMIN_AUTH_DURATION_MS: u64 = 5_000; // 5 seconds

/// The timeout for connecting to Redis
const REDIS_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The DB connection type
pub type DbConn<'a> = PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The DB pool type
pub type DbPool = Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The API key cache type
pub type ApiKeyCache = Arc<RwLock<UnboundCache<Uuid, ApiKey>>>;

/// The server struct that holds all the necessary components
#[derive(Clone)]
pub struct Server {
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
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
    /// The Arbitrum client
    pub arbitrum_client: ArbitrumClient,
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

impl Server {
    /// Create a new server instance
    pub async fn new(args: Cli, system_clock: &SystemClock) -> Result<Self, AuthServerError> {
        configure_telemtry_from_args(&args)?;
        setup_token_mapping(&args).await?;

        // Create the arbitrum client
        let arbitrum_client = create_arbitrum_client(
            args.darkpool_address.clone(),
            args.chain_id,
            args.rpc_url.clone(),
        )
        .await
        .expect("failed to create arbitrum client");

        // Set the external match fees & protocol fee
        set_external_match_fees(&arbitrum_client).await?;

        // Setup the DB connection pool
        let db_pool = Arc::new(create_db_pool(&args.database_url).await?);

        // Setup the Redis connection manager
        let redis_client = create_redis_client(&args.redis_url).await?;

        let (encryption_key, management_key, relayer_admin_key, gas_sponsor_auth_key) =
            parse_auth_server_keys(&args)?;

        let rate_limiter = AuthServerRateLimiter::new(
            args.quote_rate_limit,
            args.bundle_rate_limit,
            args.shared_bundle_rate_limit,
            args.max_gas_sponsorship_value,
        );

        let price_reporter_client =
            Arc::new(PriceReporterClient::new(args.price_reporter_url.clone())?);

        // Setup quote metrics
        let quote_metrics = maybe_setup_quote_metrics(
            &args,
            arbitrum_client.clone(),
            price_reporter_client.clone(),
        );

        let gas_sponsor_address = parse_gas_sponsor_address(&args)?;

        let gas_cost_sampler = Arc::new(
            GasCostSampler::new(
                arbitrum_client.client().clone(),
                gas_sponsor_address,
                system_clock,
            )
            .await?,
        );

        // Create the shared in-memory bundle store
        let bundle_store = BundleStore::new();

        // Start the on-chain event listener
        let chain_listener_config = OnChainEventListenerConfig {
            websocket_addr: args.eth_websocket_addr.clone(),
            arbitrum_client: arbitrum_client.clone(),
        };
        let mut chain_listener = OnChainEventListener::new(
            chain_listener_config,
            bundle_store.clone(),
            rate_limiter.clone(),
        )
        .expect("failed to build on-chain event listener");
        chain_listener.start().expect("failed to start on-chain event listener");
        chain_listener.watch();

        Ok(Self {
            db_pool,
            redis_client,
            relayer_url: args.relayer_url,
            relayer_admin_key,
            management_key,
            encryption_key,
            api_key_cache: Arc::new(RwLock::new(UnboundCache::new())),
            client: Client::new(),
            arbitrum_client,
            rate_limiter,
            quote_metrics,
            metrics_sampling_rate: args
                .metrics_sampling_rate
                .unwrap_or(1.0 /* default no sampling */),
            gas_sponsor_address,
            gas_sponsor_auth_key,
            price_reporter_client,
            gas_cost_sampler,
            min_sponsored_order_quote_amount: args.min_sponsored_order_quote_amount,
            bundle_store,
        })
    }

    /// Get a db connection from the pool
    pub async fn get_db_conn(&self) -> Result<DbConn, AuthServerError> {
        self.db_pool.get().await.map_err(AuthServerError::db)
    }

    /// Send a proxied request to the relayer with admin authentication
    pub(crate) async fn send_admin_request(
        &self,
        method: Method,
        path: &str,
        mut headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response<Bytes>, ApiError> {
        // Ensure that the content-length header is set correctly
        // so that the relayer can deserialize the proxied request
        headers.insert(CONTENT_LENGTH, body.len().into());

        // Admin authenticate the request
        self.admin_authenticate(path, &mut headers, &body)?;

        // Forward the request to the relayer
        let url = format!("{}{}", self.relayer_url, path);
        let req = self.client.request(method, &url).headers(headers).body(body);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers().clone();
                let body = resp.bytes().await.map_err(|e| {
                    ApiError::internal(format!("Failed to read response body: {e}"))
                })?;

                let mut response = warp::http::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;

                Ok(response)
            },
            Err(e) => {
                error!("Error proxying request: {}", e);
                Err(ApiError::internal(e))
            },
        }
    }

    /// Admin authenticate a request
    pub fn admin_authenticate(
        &self,
        path: &str,
        headers: &mut HeaderMap,
        body: &[u8],
    ) -> Result<(), ApiError> {
        let key = self.relayer_admin_key;
        let expiration = Duration::from_millis(ADMIN_AUTH_DURATION_MS);
        add_expiring_auth_to_headers(path, headers, body, &key, expiration);
        Ok(())
    }

    // --- Rate Limiting --- //

    /// Check the quote rate limiter
    pub async fn check_quote_rate_limit(&self, key_description: String) -> Result<(), ApiError> {
        if !self.rate_limiter.check_quote_token(key_description.clone()).await {
            warn!("Quote rate limit exceeded for key: {key_description}");
            return Err(ApiError::TooManyRequests);
        }
        Ok(())
    }

    /// Check the bundle rate limiter
    pub async fn check_bundle_rate_limit(
        &self,
        key_description: String,
        shared: bool,
    ) -> Result<(), ApiError> {
        if !self.rate_limiter.check_bundle_token(key_description.clone(), shared).await {
            warn!("Bundle rate limit exceeded for key: {key_description}");
            return Err(ApiError::TooManyRequests);
        }
        Ok(())
    }

    /// Check the gas sponsorship rate limiter
    ///
    /// Returns a boolean indicating whether or not the gas sponsorship rate
    /// limit has been exceeded.
    pub async fn check_gas_sponsorship_rate_limit(&self, key_description: String) -> bool {
        if !self.rate_limiter.check_gas_sponsorship(key_description.clone()).await {
            warn!(
                key_description = key_description.as_str(),
                "Gas sponsorship rate limit exceeded for key: {key_description}"
            );
            return false;
        }
        true
    }

    // --- Caching --- //

    /// Check the cache for an API key
    pub async fn get_cached_api_secret(&self, id: Uuid) -> Option<ApiKey> {
        let cache = self.api_key_cache.read().await;
        cache.get_store().get(&id).cloned()
    }

    /// Cache an API key
    pub async fn cache_api_key(&self, api_key: ApiKey) {
        let mut cache = self.api_key_cache.write().await;
        cache.cache_set(api_key.id, api_key);
    }

    /// Mark a cached API key as expired
    pub async fn mark_cached_key_expired(&self, id: Uuid) {
        let mut cache = self.api_key_cache.write().await;
        if let Some(key) = cache.cache_get_mut(&id) {
            key.is_active = false;
        }
    }

    /// Determines if the current request should be sampled for metrics
    /// collection
    pub fn should_sample_metrics(&self) -> bool {
        rand::thread_rng().gen_bool(self.metrics_sampling_rate)
    }
}

/// Configure telemetry from the command line arguments
fn configure_telemtry_from_args(args: &Cli) -> Result<(), AuthServerError> {
    configure_telemetry(
        args.datadog_enabled, // datadog_enabled
        false,                // otlp_enabled
        args.metrics_enabled, // metrics_enabled
        "".to_string(),       // collector_endpoint
        &args.statsd_host,    // statsd_host
        args.statsd_port,     // statsd_port
    )
    .map_err(AuthServerError::setup)
}

/// Setup the token mapping
async fn setup_token_mapping(args: &Cli) -> Result<(), AuthServerError> {
    let chain_id = args.chain_id;
    let token_remap_file = args.token_remap_file.clone();
    tokio::task::spawn_blocking(move || setup_token_remaps(token_remap_file, chain_id))
        .await
        .unwrap()
        .map_err(AuthServerError::setup)
}

/// Set the external match fees & protocol fee
async fn set_external_match_fees(arbitrum_client: &ArbitrumClient) -> Result<(), AuthServerError> {
    let protocol_fee = arbitrum_client.get_protocol_fee().await.map_err(AuthServerError::setup)?;

    PROTOCOL_FEE
        .set(protocol_fee)
        .map_err(|_| AuthServerError::setup("Failed to set protocol fee"))?;

    let tokens: Vec<Token> = get_all_tokens()
        .into_iter()
        .chain(iter::once(Token::from_addr(NATIVE_ASSET_ADDRESS)))
        .collect();

    for token in tokens {
        // Fetch the fee override from the contract
        let addr = token.get_ethers_address();
        let fee =
            arbitrum_client.get_external_match_fee(addr).await.map_err(AuthServerError::setup)?;

        // Write the fee into the mapping
        let addr_bigint = token.get_addr_biguint();
        set_external_match_fee(&addr_bigint, fee);
    }

    Ok(())
}

/// Parse the encryption key, management key, relayer admin key, and gas sponsor
/// auth key
fn parse_auth_server_keys(
    args: &Cli,
) -> Result<(Aes128Gcm, HmacKey, HmacKey, SigningKey), AuthServerError> {
    let encryption_key_bytes =
        general_purpose::STANDARD.decode(&args.encryption_key).map_err(AuthServerError::setup)?;

    let encryption_key =
        Aes128Gcm::new_from_slice(&encryption_key_bytes).map_err(AuthServerError::setup)?;

    let management_key =
        HmacKey::from_base64_string(&args.management_key).map_err(AuthServerError::setup)?;

    let relayer_admin_key =
        HmacKey::from_base64_string(&args.relayer_admin_key).map_err(AuthServerError::setup)?;

    let gas_sponsor_auth_key_bytes =
        hex::decode(&args.gas_sponsor_auth_key).map_err(AuthServerError::setup)?;

    let gas_sponsor_auth_key =
        SigningKey::from_slice(&gas_sponsor_auth_key_bytes).map_err(AuthServerError::setup)?;

    Ok((encryption_key, management_key, relayer_admin_key, gas_sponsor_auth_key))
}

/// Create a database pool
pub async fn create_db_pool(db_url: &str) -> Result<DbPool, AuthServerError> {
    let mut conf = ManagerConfig::default();
    conf.custom_setup = Box::new(move |url| Box::pin(establish_connection(url)));

    let manager = AsyncDieselConnectionManager::new_with_config(db_url, conf);
    Pool::builder().build(manager).await.map_err(AuthServerError::db)
}

/// Establish a connection to the database
pub async fn establish_connection(db_url: &str) -> Result<AsyncPgConnection, ConnectionError> {
    // Build a TLS connector, we don't validate certificates for simplicity.
    // Practically this is unnecessary because we will be limiting our traffic to
    // within a siloed environment when deployed
    let connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("failed to build tls connector");
    let connector = MakeTlsConnector::new(connector);
    let (client, conn) = tokio_postgres::connect(db_url, connector)
        .await
        .map_err(|e| ConnectionError::BadConnection(e.to_string()))?;

    // Spawn the connection handle in a separate task
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            error!("Connection error: {}", e);
        }
    });

    AsyncPgConnection::try_from(client).await
}

/// Create a Redis client.
/// Under the hood, this uses a `ConnectionManager` to manage a single,
/// shareable connection to Redis. This will automatically reconnect if the
/// connection is lost.
async fn create_redis_client(redis_url: &str) -> Result<ConnectionManager, AuthServerError> {
    let client = redis::Client::open(redis_url).map_err(AuthServerError::redis)?;
    tokio::time::timeout(REDIS_CONNECT_TIMEOUT, ConnectionManager::new(client))
        .await
        .map_err(AuthServerError::setup)?
        .map_err(AuthServerError::setup)
}

/// Setup the quote metrics recorder and sources if enabled
fn maybe_setup_quote_metrics(
    args: &Cli,
    arbitrum_client: ArbitrumClient,
    price_reporter: Arc<PriceReporterClient>,
) -> Option<Arc<QuoteComparisonHandler>> {
    if !args.enable_quote_comparison {
        return None;
    }

    let odos_source = QuoteSource::odos_default();
    Some(Arc::new(QuoteComparisonHandler::new(vec![odos_source], arbitrum_client, price_reporter)))
}

/// Parse the gas sponsor address from the CLI args
fn parse_gas_sponsor_address(args: &Cli) -> Result<Address, AuthServerError> {
    let gas_sponsor_address_bytes =
        hex::decode(&args.gas_sponsor_address).map_err(AuthServerError::setup)?;

    Ok(Address::from_slice(&gas_sponsor_address_bytes))
}
