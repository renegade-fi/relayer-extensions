//! Defines the server struct and associated functions
//!
//! The server is a dependency injection container for the authentication server
mod api_auth;
mod handle_external_match;
mod handle_key_management;
mod helpers;
mod queries;

use crate::{error::AuthServerError, models::ApiKey, relayer_client::RelayerClient, ApiError, Cli};
use base64::{engine::general_purpose, Engine};
use bb8::{Pool, PooledConnection};
use bytes::Bytes;
use cached::{Cached, UnboundCache};
use diesel::ConnectionError;
use diesel_async::{
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig},
    AsyncPgConnection,
};
use http::{HeaderMap, Method, Response};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use renegade_api::auth::add_expiring_auth_to_headers;
use renegade_common::types::wallet::keychain::HmacKey;
use reqwest::Client;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::error;
use uuid::Uuid;

/// The duration for which the admin authentication is valid
const ADMIN_AUTH_DURATION_MS: u64 = 5_000; // 5 seconds

/// The DB connection type
pub type DbConn<'a> = PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The DB pool type
pub type DbPool = Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The API key cache type
pub type ApiKeyCache = Arc<RwLock<UnboundCache<Uuid, ApiKey>>>;

/// The server struct that holds all the necessary components
pub struct Server {
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
    /// The URL of the relayer
    pub relayer_url: String,
    /// The admin key for the relayer
    pub relayer_admin_key: HmacKey,
    /// The management key for the auth server
    pub management_key: HmacKey,
    /// The encryption key for storing API secrets
    pub encryption_key: Vec<u8>,
    /// The api key cache
    pub api_key_cache: ApiKeyCache,
    /// The HTTP client
    pub client: Client,
    /// A client for interacting with the relayer
    pub relayer_client: RelayerClient,
}

impl Server {
    /// Create a new server instance
    pub async fn new(args: Cli) -> Result<Self, AuthServerError> {
        // Setup the DB connection pool
        let db_pool = create_db_pool(&args.database_url).await?;

        // Parse the decryption key, management key, and relayer admin key as
        // base64 encoded strings
        let encryption_key = general_purpose::STANDARD
            .decode(&args.encryption_key)
            .map_err(AuthServerError::encryption)?;
        let management_key =
            HmacKey::from_base64_string(&args.management_key).map_err(AuthServerError::setup)?;
        let relayer_admin_key =
            HmacKey::from_base64_string(&args.relayer_admin_key).map_err(AuthServerError::setup)?;

        Ok(Self {
            db_pool: Arc::new(db_pool),
            relayer_url: args.relayer_url.clone(),
            relayer_admin_key,
            management_key,
            encryption_key,
            api_key_cache: Arc::new(RwLock::new(UnboundCache::new())),
            client: Client::new(),
            relayer_client: RelayerClient::new(&args.relayer_url),
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
