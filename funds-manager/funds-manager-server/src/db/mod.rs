//! Database code

use bb8::{Pool, PooledConnection};
use diesel::ConnectionError;
use diesel_async::{
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig},
    AsyncPgConnection,
};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use renegade_util::err_str;
use tracing::error;

use crate::error::FundsManagerError;

pub mod models;
#[allow(missing_docs)]
pub mod schema;

/// The DB connection type
pub type DbConn<'a> = PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;
/// The DB pool type
pub type DbPool = Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;

/// Create a connection pool for the given database url
pub async fn create_db_pool(db_url: &str) -> Result<DbPool, FundsManagerError> {
    let mut conf = ManagerConfig::default();
    conf.custom_setup = Box::new(move |url| Box::pin(establish_connection(url)));

    let manager = AsyncDieselConnectionManager::new_with_config(db_url, conf);
    Pool::builder().build(manager).await.map_err(err_str!(FundsManagerError::Db))
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
        .map_err(err_str!(ConnectionError::BadConnection))?;

    // Spawn the connection handle in a separate task
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            error!("Connection error: {}", e);
        }
    });

    AsyncPgConnection::try_from(client).await
}
