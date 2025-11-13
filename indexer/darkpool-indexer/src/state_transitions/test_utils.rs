//! Common utilities for state transition tests

use alloy::primitives::Address;
use postgresql_embedded::PostgreSQL;
use rand::thread_rng;
use renegade_circuit_types::csprng::PoseidonCSPRNG;
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::{
    db::test_utils::setup_test_db,
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::MasterViewSeed,
};

// ----------------------
// | Test Setup Helpers |
// ----------------------

/// Set up a state applicator targeting a local PostgreSQL instance
pub async fn setup_test_state_applicator()
-> Result<(StateApplicator, PostgreSQL), StateTransitionError> {
    let (db_client, postgres) = setup_test_db().await?;
    let applicator = StateApplicator::new(db_client);

    Ok((applicator, postgres))
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

/// Compute the first recovery ID of the nth expected state object for the given
/// master view seed.
pub fn get_expected_object_recovery_id(
    master_view_seed: &MasterViewSeed,
    object_number: u64,
) -> Scalar {
    let recovery_stream_seed = master_view_seed.recovery_seed_csprng.get_ith(object_number);
    let recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
    recovery_stream.get_ith(0)
}

// --------------------------
// | Test Assertion Helpers |
// --------------------------

/// Assert that a CSPRNG is in the expected state
pub fn assert_csprng_state(csprng: &PoseidonCSPRNG, expected_seed: Scalar, expected_index: u64) {
    assert_eq!(csprng.seed, expected_seed);
    assert_eq!(csprng.index, expected_index);
}
