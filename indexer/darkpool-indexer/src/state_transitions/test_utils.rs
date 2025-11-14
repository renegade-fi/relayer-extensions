//! Common utilities for state transition tests

use alloy::primitives::Address;
use darkpool_indexer_api::types::sqs::MasterViewSeedMessage;
use postgresql_embedded::PostgreSQL;
use rand::{Rng, thread_rng};
use renegade_circuit_types::{
    Amount, balance::Balance, csprng::PoseidonCSPRNG, state_wrapper::StateWrapper,
};
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::{
    db::{client::DbClient, error::DbError, test_utils::setup_test_db},
    state_transitions::{
        StateApplicator, create_balance::CreateBalanceTransition, deposit::DepositTransition,
        error::StateTransitionError,
    },
    types::{ExpectedStateObject, MasterViewSeed},
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

/// Sets up an expected state object in the DB, generating a new master view
/// seed for the account owning the state object.
///
/// Returns the expected state object.
pub async fn setup_expected_state_object(
    state_applicator: &StateApplicator,
) -> Result<ExpectedStateObject, StateTransitionError> {
    let mut master_view_seed = gen_random_master_view_seed();

    let master_view_seed_message = MasterViewSeedMessage {
        account_id: master_view_seed.account_id,
        owner_address: master_view_seed.owner_address,
        seed: master_view_seed.seed,
    };

    state_applicator.register_master_view_seed(master_view_seed_message).await?;

    Ok(master_view_seed.next_expected_state_object())
}

/// Generate the state transition which should result in the given
/// expected state object being indexed as a new balance.
///
/// Returns the create balance transition, along with the expected balance
/// object.
pub fn gen_create_balance_transition(
    expected_state_object: &ExpectedStateObject,
) -> (CreateBalanceTransition, StateWrapper<Balance>) {
    let mint = Address::random();
    let owner = Address::random();
    let relayer_fee_recipient = Address::random();
    let one_time_authority = Address::random();

    let balance = Balance::new(mint, owner, relayer_fee_recipient, one_time_authority);

    let mut wrapped_balance = StateWrapper::new(
        balance,
        expected_state_object.share_stream_seed,
        expected_state_object.recovery_stream_seed,
    );

    // We progress the balance's recovery stream to represent the computation of the
    // 0th recovery ID
    wrapped_balance.recovery_stream.advance_by(1);

    let transition = CreateBalanceTransition {
        recovery_id: expected_state_object.recovery_id,
        block_number: 0,
        public_share: wrapped_balance.public_share(),
    };

    (transition, wrapped_balance)
}

/// Generate the state transition which should result in the given
/// balance being updated with a deposit.
///
/// Returns the deposit transition, along with the updated balance.
pub fn gen_deposit_transition(
    initial_balance: &StateWrapper<Balance>,
) -> (DepositTransition, StateWrapper<Balance>) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Advance the recovery stream to indicate the next object version
    updated_balance.recovery_stream.advance_by(1);

    // Apply a random deposit amount to the balance
    let deposit_amount: Amount = thread_rng().r#gen();
    updated_balance.inner.amount += deposit_amount;

    // We re-encrypt only the updated shares of the balance, which in this case
    // pertain only to the amount
    let new_amount = updated_balance.inner.amount;
    let new_amount_public_share = updated_balance.stream_cipher_encrypt(&new_amount);

    // Update the public share of the balance
    let mut public_share = updated_balance.public_share();
    public_share.amount = new_amount_public_share;
    updated_balance.public_share = public_share;

    // Construct the associated nullifier spend data
    let transition =
        DepositTransition { nullifier: spent_nullifier, block_number: 0, new_amount_public_share };

    (transition, updated_balance)
}

// ---------------------------
// | Test Validation Helpers |
// ---------------------------

/// Assert that a CSPRNG is in the expected state
pub fn assert_csprng_state(csprng: &PoseidonCSPRNG, expected_seed: Scalar, expected_index: u64) {
    assert_eq!(csprng.seed, expected_seed);
    assert_eq!(csprng.index, expected_index);
}

/// Validate the indexing of a balance object against the expected
/// circuit type
pub async fn validate_balance_indexing(
    db_client: &DbClient,
    expected_balance: &StateWrapper<Balance>,
) -> Result<(), DbError> {
    let mut conn = db_client.get_db_conn().await?;

    let nullifier = expected_balance.compute_nullifier();
    let indexed_balance = db_client.get_balance_by_nullifier(nullifier, &mut conn).await?;

    // Assert that the indexed balance matches the expected balance.
    // This covers the CSPRNG states, inner circuit type, and public shares.
    assert_eq!(&indexed_balance.balance, expected_balance);

    Ok(())
}
