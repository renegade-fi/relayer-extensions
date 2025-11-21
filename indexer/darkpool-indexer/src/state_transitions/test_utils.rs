//! Common utilities for state transition tests

use alloy::primitives::{Address, B256};
use darkpool_indexer_api::types::sqs::MasterViewSeedMessage;
use postgresql_embedded::PostgreSQL;
use rand::{Rng, distributions::uniform::SampleRange, thread_rng};
use renegade_circuit_types::{
    Amount, balance::Balance, csprng::PoseidonCSPRNG, fixed_point::FixedPoint, intent::Intent,
    max_amount, state_wrapper::StateWrapper,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;
use uuid::Uuid;

use crate::{
    db::{client::DbClient, error::DbError, test_utils::setup_test_db},
    state_transitions::{
        StateApplicator,
        create_balance::CreateBalanceTransition,
        create_intent::{CreateIntentTransition, IntentCreationData},
        create_public_intent::CreatePublicIntentTransition,
        deposit::DepositTransition,
        error::StateTransitionError,
        pay_protocol_fee::PayProtocolFeeTransition,
        pay_relayer_fee::PayRelayerFeeTransition,
        settle_match_into_balance::{BalanceSettlementData, SettleMatchIntoBalanceTransition},
        settle_match_into_intent::{IntentSettlementData, SettleMatchIntoIntentTransition},
        settle_match_into_public_intent::SettleMatchIntoPublicIntentTransition,
        withdraw::WithdrawTransition,
    },
    types::{BalanceSharesInMatch, ExpectedStateObject, MasterViewSeed, PublicIntentStateObject},
};

// -------------
// | Constants |
// -------------

/// The relayer fee used for testing, as an f64
const RELAYER_FEE_F64: f64 = 0.001; // 1bp
/// The protocol fee used for testing, as an f64
const PROTOCOL_FEE_F64: f64 = 0.001; // 1bp

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

/// Get the relayer fee as a fixed point
pub fn relayer_fee() -> FixedPoint {
    FixedPoint::from_f64_round_down(RELAYER_FEE_F64)
}

/// Get the protocol fee as a fixed point
pub fn protocol_fee() -> FixedPoint {
    FixedPoint::from_f64_round_down(PROTOCOL_FEE_F64)
}

/// Generate a random amount valid in a wallet
///
/// Leave buffer for additions and subtractions
pub fn random_amount() -> Amount {
    let mut rng = thread_rng();
    let amt = (0..max_amount()).sample_single(&mut rng);

    amt / 10
}

/// Add two amounts, saturating up to the maximum amount
fn add_up_to_max(amount_a: Amount, amount_b: Amount) -> Amount {
    amount_a.saturating_add(amount_b).min(max_amount())
}

/// Generate a random master view seed
pub fn gen_random_master_view_seed() -> MasterViewSeed {
    let account_id = Uuid::new_v4();
    let owner_address = Address::random();
    let seed = Scalar::random(&mut thread_rng());

    MasterViewSeed::new(account_id, owner_address, seed)
}

/// Generate a random intent for the given owner
pub fn gen_random_intent(owner: Address) -> Intent {
    let in_token = Address::random();
    let out_token = Address::random();
    let min_price = FixedPoint::from_f64_round_down(thread_rng().r#gen());
    let amount_in = random_amount();

    Intent { in_token, out_token, owner, min_price, amount_in }
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

/// Generates a random master view seed and registers it via the state
/// applicator.
///
/// Returns the master view seed.
pub async fn register_random_master_view_seed(
    state_applicator: &StateApplicator,
) -> Result<MasterViewSeed, StateTransitionError> {
    let master_view_seed = gen_random_master_view_seed();

    let master_view_seed_message = MasterViewSeedMessage {
        account_id: master_view_seed.account_id,
        owner_address: master_view_seed.owner_address,
        seed: master_view_seed.seed,
    };

    state_applicator.register_master_view_seed(master_view_seed_message).await?;

    Ok(master_view_seed)
}

/// Sets up an expected state object in the DB, generating a new master view
/// seed for the account owning the state object.
///
/// Returns the expected state object.
pub async fn setup_expected_state_object(
    state_applicator: &StateApplicator,
) -> Result<ExpectedStateObject, StateTransitionError> {
    let mut master_view_seed = register_random_master_view_seed(state_applicator).await?;
    Ok(master_view_seed.next_expected_state_object())
}

/// Generate the state transition which should result in the given
/// expected state object being indexed as a new balance.
///
/// We create the balance with random fees & amounts for convenience in tests.
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
    let relayer_fee_balance = random_amount();
    let protocol_fee_balance = random_amount();
    let amount = random_amount();

    let balance = Balance {
        mint,
        owner,
        one_time_authority,
        relayer_fee_recipient,
        relayer_fee_balance,
        protocol_fee_balance,
        amount,
    };

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
/// expected state object being indexed as a new intent.
///
/// Returns the create intent transition, along with the expected intent
/// object.
pub fn gen_create_intent_transition(
    expected_state_object: &ExpectedStateObject,
) -> (CreateIntentTransition, StateWrapper<Intent>) {
    let owner = Address::random();

    let intent = gen_random_intent(owner);

    let mut wrapped_intent = StateWrapper::new(
        intent,
        expected_state_object.share_stream_seed,
        expected_state_object.recovery_stream_seed,
    );

    // We progress the balance's recovery stream to represent the computation of the
    // 0th recovery ID
    wrapped_intent.recovery_stream.advance_by(1);

    // For now, we simply use the `NewIntentShare` variant of the intent creation
    // data
    let intent_creation_data = IntentCreationData::NewIntentShare(wrapped_intent.public_share());

    let transition = CreateIntentTransition {
        recovery_id: expected_state_object.recovery_id,
        block_number: 0,
        intent_creation_data,
    };

    (transition, wrapped_intent)
}

/// Update the amount of a balance.
///
/// Returns the public share of the new amount.
fn update_balance_amount(balance: &mut StateWrapper<Balance>, new_amount: Amount) -> Scalar {
    // Advance the recovery stream to indicate the next object version
    balance.recovery_stream.advance_by(1);

    // Update the balance amount
    balance.inner.amount = new_amount;

    // We re-encrypt only the updated shares of the balance, which in this case
    // pertain only to the amount
    let new_amount_public_share = balance.stream_cipher_encrypt(&new_amount);

    // Update the public share of the balance
    let mut public_share = balance.public_share();
    public_share.amount = new_amount_public_share;
    balance.public_share = public_share;

    new_amount_public_share
}

/// Update the amount & fees in a balance.
///
/// Returns the public shares of the new fees & amount.
fn update_balance_amount_and_fees(
    balance: &mut StateWrapper<Balance>,
    new_relayer_fee_balance: Amount,
    new_protocol_fee_balance: Amount,
    new_amount: Amount,
) -> (Scalar, Scalar, Scalar) {
    // Advance the recovery stream to indicate the next object version
    balance.recovery_stream.advance_by(1);

    // Update the balance fees & amount
    balance.inner.relayer_fee_balance = new_relayer_fee_balance;
    balance.inner.protocol_fee_balance = new_protocol_fee_balance;
    balance.inner.amount = new_amount;

    // We re-encrypt only the updated shares of the balance
    let new_relayer_fee_public_share = balance.stream_cipher_encrypt(&new_relayer_fee_balance);
    let new_protocol_fee_public_share = balance.stream_cipher_encrypt(&new_protocol_fee_balance);
    let new_amount_public_share = balance.stream_cipher_encrypt(&new_amount);

    // Update the public share of the balance
    let mut public_share = balance.public_share();

    public_share.relayer_fee_balance = new_relayer_fee_public_share;
    public_share.protocol_fee_balance = new_protocol_fee_public_share;
    public_share.amount = new_amount_public_share;

    balance.public_share = public_share;

    (new_relayer_fee_public_share, new_protocol_fee_public_share, new_amount_public_share)
}

/// Update the protocol fee in a balance.
///
/// Returns the public share of the new protocol fee.
fn update_balance_protocol_fee(
    balance: &mut StateWrapper<Balance>,
    new_protocol_fee_balance: Amount,
) -> Scalar {
    // Advance the recovery stream to indicate the next object version
    balance.recovery_stream.advance_by(1);

    // Update the protocol fee
    balance.inner.protocol_fee_balance = new_protocol_fee_balance;

    // We re-encrypt only the updated protocol fee
    let new_protocol_fee_public_share = balance.stream_cipher_encrypt(&new_protocol_fee_balance);

    // Update the public share of the balance
    let mut public_share = balance.public_share();
    public_share.protocol_fee_balance = new_protocol_fee_public_share;
    balance.public_share = public_share;

    new_protocol_fee_public_share
}

/// Update the relayer fee in a balance.
///
/// Returns the public share of the new relayer fee.
fn update_balance_relayer_fee(
    balance: &mut StateWrapper<Balance>,
    new_relayer_fee_balance: Amount,
) -> Scalar {
    // Advance the recovery stream to indicate the next object version
    balance.recovery_stream.advance_by(1);

    // Update the relayer fee
    balance.inner.relayer_fee_balance = new_relayer_fee_balance;

    // We re-encrypt only the updated relayer fee
    let new_relayer_fee_public_share = balance.stream_cipher_encrypt(&new_relayer_fee_balance);

    // Update the public share of the balance
    let mut public_share = balance.public_share();
    public_share.relayer_fee_balance = new_relayer_fee_public_share;
    balance.public_share = public_share;

    new_relayer_fee_public_share
}

/// Update the amount in an intent.
///
/// Returns the public share of the new amount.
fn update_intent_amount(intent: &mut StateWrapper<Intent>, new_amount: Amount) -> Scalar {
    // Advance the recovery stream to indicate the next object version
    intent.recovery_stream.advance_by(1);

    // Update the intent amount
    intent.inner.amount_in = new_amount;

    // Re-encrypt the updated amount share
    let new_amount_public_share = intent.stream_cipher_encrypt(&new_amount);

    // Update the public share of the intent
    let mut public_share = intent.public_share();
    public_share.amount_in = new_amount_public_share;
    intent.public_share = public_share;

    new_amount_public_share
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

    // Apply a random deposit amount to the balance
    let new_amount = add_up_to_max(initial_balance.inner.amount, random_amount());
    let new_amount_public_share = update_balance_amount(&mut updated_balance, new_amount);

    // Construct the associated deposit transition
    let transition =
        DepositTransition { nullifier: spent_nullifier, block_number: 0, new_amount_public_share };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// balance being updated with a withdrawal.
///
/// Returns the withdrawal transition, along with the updated balance.
pub fn gen_withdraw_transition(
    initial_balance: &StateWrapper<Balance>,
) -> (WithdrawTransition, StateWrapper<Balance>) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Apply a random withdrawal amount to the balance
    let new_amount = initial_balance.inner.amount.saturating_sub(random_amount());
    let new_amount_public_share = update_balance_amount(&mut updated_balance, new_amount);

    // Construct the associated withdrawal transition
    let transition =
        WithdrawTransition { nullifier: spent_nullifier, block_number: 0, new_amount_public_share };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// balance being updated with a protocol fee payment.
///
/// Returns the protocol fee payment transition, along with the updated balance.
pub fn gen_pay_protocol_fee_transition(
    initial_balance: &StateWrapper<Balance>,
) -> (PayProtocolFeeTransition, StateWrapper<Balance>) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Apply a random fee payment to the balance
    let new_protocol_fee_balance =
        initial_balance.inner.protocol_fee_balance.saturating_sub(random_amount());

    let new_protocol_fee_public_share =
        update_balance_protocol_fee(&mut updated_balance, new_protocol_fee_balance);

    // Construct the associated fee payment transition
    let transition = PayProtocolFeeTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        new_protocol_fee_public_share,
    };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// balance being updated with a relayer fee payment.
///
/// Returns the relayer fee payment transition, along with the updated balance.
pub fn gen_pay_relayer_fee_transition(
    initial_balance: &StateWrapper<Balance>,
) -> (PayRelayerFeeTransition, StateWrapper<Balance>) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Apply a random fee payment to the balance
    let new_relayer_fee_balance =
        initial_balance.inner.relayer_fee_balance.saturating_sub(random_amount());

    let new_relayer_fee_public_share =
        update_balance_relayer_fee(&mut updated_balance, new_relayer_fee_balance);

    // Construct the associated fee payment transition
    let transition = PayRelayerFeeTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        new_relayer_fee_public_share,
    };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// balance being updated with a match settlement.
///
/// Returns the match settlement transition, along with the updated balance.
pub fn gen_settle_match_into_balance_transition(
    initial_balance: &StateWrapper<Balance>,
) -> (SettleMatchIntoBalanceTransition, StateWrapper<Balance>) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Apply a random match receive amount to the balance
    let match_amount = random_amount();

    let relayer_fee_amount = scalar_to_u128(&relayer_fee().floor_mul_int(match_amount));
    let protocol_fee_amount = scalar_to_u128(&protocol_fee().floor_mul_int(match_amount));
    let net_match_amount =
        match_amount.saturating_sub(relayer_fee_amount).saturating_sub(protocol_fee_amount);

    let new_amount = add_up_to_max(initial_balance.inner.amount, net_match_amount);
    let new_relayer_fee_balance =
        add_up_to_max(initial_balance.inner.relayer_fee_balance, relayer_fee_amount);
    let new_protocol_fee_balance =
        add_up_to_max(initial_balance.inner.protocol_fee_balance, protocol_fee_amount);

    let (relayer_fee_public_share, protocol_fee_public_share, amount_public_share) =
        update_balance_amount_and_fees(
            &mut updated_balance,
            new_relayer_fee_balance,
            new_protocol_fee_balance,
            new_amount,
        );

    // For now, we simply use the `PrivateFill` variant of the balance settlement
    // data
    let balance_settlement_data = BalanceSettlementData::PrivateFill(BalanceSharesInMatch {
        relayer_fee_public_share,
        protocol_fee_public_share,
        amount_public_share,
    });

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoBalanceTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        balance_settlement_data,
    };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// owner creating a new public intent
pub fn gen_create_public_intent_transition(owner: Address) -> CreatePublicIntentTransition {
    let intent = gen_random_intent(owner);

    // Create a dummy intent hash, we don't need to actually hash the intent for
    // testing
    let intent_hash = B256::random();

    CreatePublicIntentTransition { intent, intent_hash, block_number: 0 }
}

/// Generate the state transition which should result in the given
/// public intent being updated with a match settlement.
///
/// Returns the match settlement transition.
pub fn gen_settle_match_into_public_intent_transition(
    initial_public_intent: &PublicIntentStateObject,
) -> SettleMatchIntoPublicIntentTransition {
    let mut updated_intent = initial_public_intent.clone();

    // Apply a random match amount to the public intent
    let match_amount = random_amount().min(initial_public_intent.intent.amount_in);
    updated_intent.intent.amount_in -= match_amount;

    updated_intent.version += 1;

    SettleMatchIntoPublicIntentTransition {
        intent_hash: updated_intent.intent_hash,
        intent: updated_intent.intent,
        version: updated_intent.version,
        block_number: 0,
    }
}

/// Generate the state transition which should result in the given
/// intent being updated with a match settlement.
///
/// Returns the match settlement transition, along with the updated intent.
pub fn gen_settle_match_into_intent_transition(
    initial_intent: &StateWrapper<Intent>,
) -> (SettleMatchIntoIntentTransition, StateWrapper<Intent>) {
    let spent_nullifier = initial_intent.compute_nullifier();

    let mut updated_intent = initial_intent.clone();

    // Apply a random match amount to the intent
    let match_amount = random_amount();

    let new_amount = initial_intent.inner.amount_in.saturating_sub(match_amount);

    let amount_public_share = update_intent_amount(&mut updated_intent, new_amount);

    // For now, we simply use the `UpdatedAmountShare` variant of the intent
    // settlement data
    let intent_settlement_data = IntentSettlementData::UpdatedAmountShare(amount_public_share);

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoIntentTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        intent_settlement_data,
    };

    (transition, updated_intent)
}

// ---------------------------
// | Test Validation Helpers |
// ---------------------------

/// Assert that a CSPRNG is in the expected state
pub fn assert_csprng_state(csprng: &PoseidonCSPRNG, expected_seed: Scalar, expected_index: u64) {
    assert_eq!(csprng.seed, expected_seed);
    assert_eq!(csprng.index, expected_index);
}

/// Validate the rotation of an account's next expected state object
pub async fn validate_expected_state_object_rotation(
    db_client: &DbClient,
    old_expected_state_object: &ExpectedStateObject,
) -> Result<(), DbError> {
    let mut conn = db_client.get_db_conn().await?;

    // Assert that the indexed master view seed's CSPRNG states are advanced
    // correctly
    let indexed_master_view_seed = db_client
        .get_master_view_seed_by_account_id(old_expected_state_object.account_id, &mut conn)
        .await?;

    let recovery_seed_stream = &indexed_master_view_seed.recovery_seed_csprng;
    assert_csprng_state(recovery_seed_stream, recovery_seed_stream.seed, 2);

    let share_seed_stream = &indexed_master_view_seed.share_seed_csprng;
    assert_csprng_state(share_seed_stream, share_seed_stream.seed, 2);

    // Assert that the next expected state object is indexed correctly
    let expected_recovery_stream_seed = recovery_seed_stream.get_ith(1);
    let expected_share_stream_seed = share_seed_stream.get_ith(1);
    let next_expected_state_object = ExpectedStateObject::new(
        indexed_master_view_seed.account_id,
        expected_recovery_stream_seed,
        expected_share_stream_seed,
    );

    let indexed_next_expected_state_object = db_client
        .get_expected_state_object(next_expected_state_object.recovery_id, &mut conn)
        .await?;

    assert_eq!(indexed_next_expected_state_object, next_expected_state_object);

    // Assert that the old expected state object is deleted
    let deleted_expected_state_object_res =
        db_client.get_expected_state_object(old_expected_state_object.recovery_id, &mut conn).await;

    assert!(matches!(
        deleted_expected_state_object_res,
        Err(DbError::DieselError(diesel::NotFound))
    ));

    Ok(())
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

/// Validate the indexing of an intent object against the expected
/// circuit type
pub async fn validate_intent_indexing(
    db_client: &DbClient,
    expected_intent: &StateWrapper<Intent>,
) -> Result<(), DbError> {
    let mut conn = db_client.get_db_conn().await?;

    let nullifier = expected_intent.compute_nullifier();
    let indexed_intent = db_client.get_intent_by_nullifier(nullifier, &mut conn).await?;

    // Assert that the indexed intent matches the expected intent.
    // This covers the CSPRNG states, inner circuit type, and public shares.
    assert_eq!(&indexed_intent.intent, expected_intent);

    Ok(())
}

/// Validate the indexing of a public intent against the expected
/// circuit type
pub async fn validate_public_intent_indexing(
    db_client: &DbClient,
    intent_hash: B256,
    expected_intent: &Intent,
) -> Result<(), DbError> {
    let mut conn = db_client.get_db_conn().await?;

    let indexed_public_intent = db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

    // Assert that the indexed public intent matches the expected intent.
    assert_eq!(&indexed_public_intent.intent, expected_intent);

    Ok(())
}
