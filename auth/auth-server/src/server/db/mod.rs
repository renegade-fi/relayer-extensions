//! Database access helpers for the auth server
#![allow(mismatched_lifetime_syntaxes)]
use std::time::Duration;

use bb8::{Pool, PooledConnection};
use diesel::ConnectionError;
use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig},
};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use redis::aio::ConnectionManager;
use tracing::error;

use crate::error::AuthServerError;

use super::Server;

pub(crate) mod models;
pub mod queries;
pub mod redis_queries;
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub(crate) mod schema;

/// The timeout for connecting to Redis
const REDIS_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The DB connection type
pub type DbConn<'a> = PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The DB pool type
pub type DbPool = Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;

impl Server {
    /// Get a db connection from the pool
    pub async fn get_db_conn(&self) -> Result<DbConn, AuthServerError> {
        self.db_pool.get().await.map_err(AuthServerError::db)
    }
}

// -----------------
// | Setup Helpers |
// -----------------

// --- Postgres Setup --- //

/// Create a database pool
pub(crate) async fn create_db_pool(db_url: &str) -> Result<DbPool, AuthServerError> {
    let mut conf = ManagerConfig::default();
    conf.custom_setup = Box::new(move |url| Box::pin(establish_connection(url)));

    let manager = AsyncDieselConnectionManager::new_with_config(db_url, conf);
    Pool::builder().build(manager).await.map_err(AuthServerError::db)
}

/// Establish a connection to the database
pub(crate) async fn establish_connection(
    db_url: &str,
) -> Result<AsyncPgConnection, ConnectionError> {
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

// --- Redis Setup --- //

/// Create a Redis client.
/// Under the hood, this uses a `ConnectionManager` to manage a single,
/// shareable connection to Redis. This will automatically reconnect if the
/// connection is lost.
pub(crate) async fn create_redis_client(
    redis_url: &str,
) -> Result<ConnectionManager, AuthServerError> {
    let client = redis::Client::open(redis_url).map_err(AuthServerError::redis)?;
    tokio::time::timeout(REDIS_CONNECT_TIMEOUT, ConnectionManager::new(client))
        .await
        .map_err(AuthServerError::setup)?
        .map_err(AuthServerError::setup)
}
