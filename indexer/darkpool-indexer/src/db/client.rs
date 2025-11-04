//! A client for interacting with the indexer's database

use std::sync::Arc;

use bb8::{Pool, PooledConnection};
use diesel::ConnectionError;
use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig},
};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tracing::error;

use crate::db::error::DbError;

// ---------
// | Types |
// ---------

/// The DB connection type
pub type DbConn<'a> = PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The DB pool type
pub type DbPool = Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;

// ----------
// | Client |
// ----------

/// A client for the indexer database
#[derive(Clone)]
pub struct DbClient {
    /// The database connection pool
    db_pool: Arc<DbPool>,
}

impl DbClient {
    /// Create a new database client using the provided database URL
    pub async fn new(db_url: &str) -> Result<Self, DbError> {
        let mut conf = ManagerConfig::default();
        conf.custom_setup = Box::new(move |url| Box::pin(Self::establish_connection(url)));

        let manager = AsyncDieselConnectionManager::new_with_config(db_url, conf);
        let db_pool = Pool::builder().build(manager).await.map_err(DbError::client_setup)?;

        Ok(Self { db_pool: Arc::new(db_pool) })
    }

    /// Establish a connection to the database
    async fn establish_connection(db_url: &str) -> Result<AsyncPgConnection, ConnectionError> {
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
                error!("Database connection error: {}", e);
            }
        });

        AsyncPgConnection::try_from(client).await
    }

    /// Get a connection from the pool
    pub async fn get_db_conn(&self) -> Result<DbConn, DbError> {
        self.db_pool.get().await.map_err(DbError::pool_connection)
    }
}
