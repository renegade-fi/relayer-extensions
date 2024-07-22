//! Database code

use diesel_async::AsyncPgConnection;
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use renegade_util::err_str;
use tracing::error;

use crate::error::FundsManagerError;

pub mod models;
#[allow(missing_docs)]
pub mod schema;

/// Establish a connection to the database
pub async fn establish_connection(db_url: &str) -> Result<AsyncPgConnection, FundsManagerError> {
    // Build a TLS connector, we don't validate certificates for simplicity.
    // Practically this is unnecessary because we will be limiting our traffic to
    // within a siloed environment when deployed
    let connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(err_str!(FundsManagerError::Db))?;
    let connector = MakeTlsConnector::new(connector);
    let (client, conn) = tokio_postgres::connect(db_url, connector)
        .await
        .map_err(err_str!(FundsManagerError::Db))?;

    // Spawn the connection handle in a separate task
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            error!("Connection error: {}", e);
        }
    });

    AsyncPgConnection::try_from(client).await.map_err(err_str!(FundsManagerError::Db))
}
