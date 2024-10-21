//! Defines the server struct and associated functions
//!
//! The server is a dependency injection container for the authentication server
use crate::Cli;
use bb8::{Pool, PooledConnection};
use diesel::ConnectionError;
use diesel_async::{
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig},
    AsyncPgConnection,
};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use reqwest::Client;
use std::sync::Arc;
use thiserror::Error;
use tracing::error;

mod handle_proxy;

/// The DB connection type
pub type DbConn<'a> = PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The DB pool type
pub type DbPool = Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;

/// Custom error type for server errors
#[derive(Error, Debug)]
pub enum ServerError {
    /// Database connection error
    #[error("Database connection error: {0}")]
    DatabaseConnectionError(String),
}

/// The server struct that holds all the necessary components
pub struct Server {
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
    /// The URL of the relayer
    pub relayer_url: String,
    /// The admin key for the relayer
    pub relayer_admin_key: String,
    /// The HTTP client
    pub client: Client,
}

impl Server {
    /// Create a new server instance
    pub async fn new(args: Cli) -> Result<Self, ServerError> {
        let db_pool = create_db_pool(&args.database_url).await?;
        Ok(Self {
            db_pool: Arc::new(db_pool),
            relayer_url: args.relayer_url,
            relayer_admin_key: args.relayer_admin_key,
            client: Client::new(),
        })
    }
}

/// Create a database pool
pub async fn create_db_pool(db_url: &str) -> Result<DbPool, ServerError> {
    let mut conf = ManagerConfig::default();
    conf.custom_setup = Box::new(move |url| Box::pin(establish_connection(url)));

    let manager = AsyncDieselConnectionManager::new_with_config(db_url, conf);
    Pool::builder()
        .build(manager)
        .await
        .map_err(|e| ServerError::DatabaseConnectionError(e.to_string()))
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
