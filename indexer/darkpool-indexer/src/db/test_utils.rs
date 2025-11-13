//! Common utilities for DB tests

use diesel::sql_query;
use diesel_async::{AsyncConnection, AsyncMigrationHarness, AsyncPgConnection, RunQueryDsl};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use postgresql_embedded::PostgreSQL;

use crate::db::{client::DbClient, error::DbError};

// -------------
// | Constants |
// -------------

/// The name of the test database
const TEST_DB_NAME: &str = "indexer_test";
/// The migrations to apply to the test database
const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

// ---------
// | Types |
// ---------

pub struct TestDbClient {
    /// The database client
    pub client: DbClient,
    /// The local PostgreSQL instance
    pub postgres: PostgreSQL,
}

impl TestDbClient {
    /// Get a reference to the database client
    pub fn get_client(&self) -> &DbClient {
        &self.client
    }
}

// -----------
// | Helpers |
// -----------

/// Set up a test database client targeting a local PostgreSQL instance
pub async fn setup_test_db_client() -> Result<TestDbClient, DbError> {
    let mut postgres = PostgreSQL::default();

    postgres.setup().await.map_err(DbError::client_setup)?;
    postgres.start().await.map_err(DbError::client_setup)?;
    postgres.create_database(TEST_DB_NAME).await.map_err(DbError::client_setup)?;
    postgres.database_exists(TEST_DB_NAME).await.map_err(DbError::client_setup)?;

    let db_url = postgres.settings().url(TEST_DB_NAME);
    let client = DbClient::new(&db_url).await.map_err(DbError::client_setup)?;

    // Apply migrations
    let conn = client.get_db_conn().await?;
    let mut harness = AsyncMigrationHarness::new(conn);
    harness.run_pending_migrations(MIGRATIONS).map_err(DbError::client_setup)?;

    Ok(TestDbClient { client, postgres })
}

/// Clean up the test database instance.
pub async fn cleanup_test_db(test_db_client: TestDbClient) -> Result<(), DbError> {
    // Drop all connections to the test database except the current one.
    // We do this here to avoid having to manually `drop` connections established
    // in tests before invoking this function.
    let drop_conns_query = r#"
        SELECT pg_terminate_backend(pid)
        FROM pg_stat_activity
        WHERE datname = $1
            AND pid <> pg_backend_pid();
    "#;

    // Create a standalone connection to the test database for executing the above
    // query
    let mut conn = create_unpooled_conn(&test_db_client).await?;

    sql_query(drop_conns_query)
        .bind::<diesel::sql_types::Text, _>(TEST_DB_NAME.to_string())
        .execute(&mut conn)
        .await?;

    // Drop the standalone connection. With this, there are no more connections open
    // to the test database.
    drop(conn);

    // Drop the test database & stop the PostgreSQL instance.
    test_db_client.postgres.drop_database(TEST_DB_NAME).await.map_err(DbError::client_setup)?;
    test_db_client.postgres.stop().await.map_err(DbError::client_setup)
}

/// Create a new connection to the test database *without* using the connection
/// pool.
///
/// This ensures that when the connection is dropped, it is terminated instead
/// of being returned to the pool.
async fn create_unpooled_conn(test_db_client: &TestDbClient) -> Result<AsyncPgConnection, DbError> {
    let db_url = test_db_client.postgres.settings().url(TEST_DB_NAME);
    AsyncPgConnection::establish(&db_url).await.map_err(DbError::client_setup)
}
