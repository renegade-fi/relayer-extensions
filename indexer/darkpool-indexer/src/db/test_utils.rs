//! Common utilities for DB tests

use alloy::primitives::Address;
use diesel::sql_query;
use diesel_async::{AsyncConnection, AsyncMigrationHarness, AsyncPgConnection, RunQueryDsl};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use postgresql_embedded::PostgreSQL;
use rand::thread_rng;
use renegade_circuit_types::csprng::PoseidonCSPRNG;
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::{
    crypto_mocks::recovery_stream::sample_nullifier,
    db::{client::DbClient, error::DbError},
    types::MasterViewSeed,
};

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

// -------------------
// | Test DB Helpers |
// -------------------

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
    test_db_client.postgres.drop_database(TEST_DB_NAME).await.map_err(DbError::custom)?;
    test_db_client.postgres.stop().await.map_err(DbError::custom)
}

/// Create a new connection to the test database *without* using the connection
/// pool.
///
/// This ensures that when the connection is dropped, it is terminated instead
/// of being returned to the pool.
async fn create_unpooled_conn(test_db_client: &TestDbClient) -> Result<AsyncPgConnection, DbError> {
    let db_url = test_db_client.postgres.settings().url(TEST_DB_NAME);
    AsyncPgConnection::establish(&db_url).await.map_err(DbError::custom)
}

// ---------------------
// | Test Data Helpers |
// ---------------------

/// Generate a random master view seed
pub fn gen_random_master_view_seed() -> MasterViewSeed {
    let account_id = Uuid::new_v4();
    let owner_address = Address::random();
    let seed = Scalar::random(&mut thread_rng());

    MasterViewSeed::new(account_id, owner_address, seed)
}

/// Compute the first nullifier of the nth expected state object for the given
/// master view seed.
pub fn get_expected_object_nullifier(
    master_view_seed: &MasterViewSeed,
    object_number: u64,
) -> Scalar {
    let recovery_stream_seed = master_view_seed.recovery_seed_csprng.get_ith(object_number);
    let recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
    sample_nullifier(&recovery_stream, 0 /* version */)
}

// --------------------------
// | Test Assertion Helpers |
// --------------------------

/// Assert that a CSPRNG is in the expected state
pub fn assert_csprng_state(csprng: &PoseidonCSPRNG, expected_seed: Scalar, expected_index: u64) {
    assert_eq!(csprng.seed, expected_seed);
    assert_eq!(csprng.index, expected_index);
}
