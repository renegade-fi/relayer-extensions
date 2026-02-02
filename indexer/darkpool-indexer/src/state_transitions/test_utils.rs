//! Common utilities for state transition tests

use alloy::primitives::{Address, B256, Bytes, TxHash, U256};
use darkpool_indexer_api::types::message_queue::{
    MasterViewSeedMessage, PublicIntentMetadataUpdateMessage,
};
use postgresql_embedded::PostgreSQL;
use rand::{Rng, thread_rng};
use renegade_circuit_types::{
    Amount, fixed_point::FixedPoint, max_amount, schnorr::SchnorrPrivateKey,
};
use renegade_circuits::test_helpers::{random_amount, random_price};
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;
use renegade_darkpool_types::{
    balance::{DarkpoolBalance, DarkpoolStateBalance},
    csprng::PoseidonCSPRNG,
    fee::FeeTake,
    intent::{DarkpoolStateIntent, Intent},
    settlement_obligation::SettlementObligation,
};
use renegade_solidity_abi::v2::IDarkpoolV2::SignatureWithNonce;
use renegade_types_account::account::order_auth::mocks::mock_public_intent_permit;
use uuid::Uuid;

use crate::{
    db::{client::DbClient, error::DbError, test_utils::setup_test_db},
    state_transitions::{
        StateApplicator,
        cancel_order::CancelOrderTransition,
        create_balance::{BalanceCreationData, CreateBalanceTransition},
        create_intent::{CreateIntentTransition, IntentCreationData},
        deposit::DepositTransition,
        error::StateTransitionError,
        pay_protocol_fee::PayProtocolFeeTransition,
        pay_relayer_fee::PayRelayerFeeTransition,
        settle_match_into_balance::{BalanceSettlementData, SettleMatchIntoBalanceTransition},
        settle_match_into_intent::{IntentSettlementData, SettleMatchIntoIntentTransition},
        settle_public_intent::{PublicIntentSettlementData, SettlePublicIntentTransition},
        withdraw::WithdrawTransition,
    },
    types::{ExpectedStateObject, MasterViewSeed, PublicIntentStateObject},
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

/// Add two amounts, saturating up to the maximum amount
fn add_up_to_max(amount_a: Amount, amount_b: Amount) -> Amount {
    amount_a.saturating_add(amount_b).min(max_amount())
}

/// Generate a random settlement obligation, representing a trade of up to
/// `max_amount`
fn gen_random_settlement_obligation(max_amount: Amount) -> SettlementObligation {
    // We bound both the input and output amounts to the given `max_amount` for
    // simplicity.
    let amount = thread_rng().gen_range(1..=max_amount);
    SettlementObligation {
        // We use random addresses as this is not yet relevent in testing code
        input_token: Address::random(),
        output_token: Address::random(),
        amount_in: amount,
        amount_out: amount,
    }
}

/// Generate a random master view seed
pub fn gen_random_master_view_seed() -> MasterViewSeed {
    let account_id = Uuid::new_v4();
    let owner_address = Address::random();
    let seed = Scalar::random(&mut thread_rng());

    MasterViewSeed::new(account_id, owner_address, seed)
}

/// Generate a random balance
pub fn gen_random_balance() -> DarkpoolBalance {
    let mint = Address::random();
    let owner = Address::random();
    let relayer_fee_recipient = Address::random();
    let authority = SchnorrPrivateKey::random().public_key();
    let relayer_fee_balance = random_amount();
    let protocol_fee_balance = random_amount();
    let amount = random_amount();

    DarkpoolBalance {
        mint,
        owner,
        authority,
        relayer_fee_recipient,
        relayer_fee_balance,
        protocol_fee_balance,
        amount,
    }
}

/// Generate a random intent for the given owner
pub fn gen_random_intent(owner: Address) -> Intent {
    let in_token = Address::random();
    let out_token = Address::random();
    let min_price = FixedPoint::from_f64_round_down(thread_rng().r#gen());
    let amount_in = random_amount();

    Intent { in_token, out_token, owner, min_price, amount_in }
}

/// Generate a mock intent signature for public intents
pub fn gen_mock_intent_signature() -> SignatureWithNonce {
    SignatureWithNonce { nonce: U256::random(), signature: Bytes::from(vec![0u8; 65]) }
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
/// expected state object being indexed as a newly-deposited balance.
///
/// Returns the create balance transition, along with the expected balance
/// object.
pub fn gen_deposit_new_balance_transition(
    expected_state_object: &ExpectedStateObject,
) -> (CreateBalanceTransition, DarkpoolStateBalance) {
    let balance = gen_random_balance();

    let mut wrapped_balance = DarkpoolStateBalance::new(
        balance,
        expected_state_object.share_stream_seed,
        expected_state_object.recovery_stream_seed,
    );

    // We progress the balance's recovery stream to represent the computation of the
    // 0th recovery ID
    wrapped_balance.recovery_stream.advance_by(1);

    let balance_creation_data =
        BalanceCreationData::DepositNewBalance { public_share: wrapped_balance.public_share() };

    let transition = CreateBalanceTransition {
        recovery_id: expected_state_object.recovery_id,
        block_number: 0,
        balance_creation_data,
    };

    (transition, wrapped_balance)
}

/// Generate the state transition which should result in the given
/// expected state object being indexed as a new output balance resulting from a
/// public-fill match settlement.
///
/// Returns the create balance transition, along with the expected balance
/// object.
pub fn gen_new_output_balance_from_public_fill_transition(
    expected_state_object: &ExpectedStateObject,
) -> (CreateBalanceTransition, DarkpoolStateBalance) {
    let balance = gen_random_balance();

    let mut wrapped_balance = DarkpoolStateBalance::new(
        balance,
        expected_state_object.share_stream_seed,
        expected_state_object.recovery_stream_seed,
    );

    // We progress the balance's recovery stream to represent the computation of the
    // 0th recovery ID
    wrapped_balance.recovery_stream.advance_by(1);

    // Compute the pre- and post-match balance shares *before* applying the
    // settlement obligation into the balance
    let balance_share = wrapped_balance.public_share();
    let pre_match_balance_share = balance_share.clone().into();
    let post_match_balance_share = balance_share.into();

    let settlement_obligation = gen_random_settlement_obligation(wrapped_balance.inner.amount);
    let relayer_fee_rate = relayer_fee();
    let protocol_fee_rate = protocol_fee();

    let relayer_fee =
        scalar_to_u128(&relayer_fee_rate.floor_mul_int(settlement_obligation.amount_out));

    let protocol_fee =
        scalar_to_u128(&protocol_fee_rate.floor_mul_int(settlement_obligation.amount_out));

    let fees = FeeTake { relayer_fee, protocol_fee };

    wrapped_balance.apply_obligation_out_balance(&settlement_obligation, &fees);

    let balance_creation_data = BalanceCreationData::NewOutputBalanceFromPublicFill {
        pre_match_balance_share,
        post_match_balance_share,
        settlement_obligation,
        relayer_fee_rate,
        protocol_fee_rate,
    };

    let transition = CreateBalanceTransition {
        recovery_id: expected_state_object.recovery_id,
        block_number: 0,
        balance_creation_data,
    };

    (transition, wrapped_balance)
}

/// Generate the state transition which should result in the given
/// expected state object being indexed as a new output balance resulting from a
/// private-fill match settlement.
///
/// Returns the create balance transition, along with the expected balance
/// object.
pub fn gen_new_output_balance_from_private_fill_transition(
    expected_state_object: &ExpectedStateObject,
) -> (CreateBalanceTransition, DarkpoolStateBalance) {
    let balance = gen_random_balance();

    let mut wrapped_balance = DarkpoolStateBalance::new(
        balance,
        expected_state_object.share_stream_seed,
        expected_state_object.recovery_stream_seed,
    );

    let settlement_obligation = gen_random_settlement_obligation(wrapped_balance.inner.amount);
    let relayer_fee_rate = relayer_fee();
    let protocol_fee_rate = protocol_fee();

    let relayer_fee =
        scalar_to_u128(&relayer_fee_rate.floor_mul_int(settlement_obligation.amount_out));

    let protocol_fee =
        scalar_to_u128(&protocol_fee_rate.floor_mul_int(settlement_obligation.amount_out));

    let fees = FeeTake { relayer_fee, protocol_fee };

    wrapped_balance.apply_obligation_out_balance(&settlement_obligation, &fees);

    // We progress the balance's recovery stream to represent the computation of the
    // 0th recovery ID
    wrapped_balance.recovery_stream.advance_by(1);

    let balance_share = wrapped_balance.public_share();
    let pre_match_balance_share = balance_share.clone().into();
    let post_match_balance_share = balance_share.into();

    let balance_creation_data = BalanceCreationData::NewOutputBalanceFromPrivateFill {
        pre_match_balance_share,
        post_match_balance_share,
    };

    let transition = CreateBalanceTransition {
        recovery_id: expected_state_object.recovery_id,
        block_number: 0,
        balance_creation_data,
    };

    (transition, wrapped_balance)
}

/// Generate the state transition which should result in the given
/// expected state object being indexed as a new intent resulting from a
/// private-fill match settlement.
///
/// Returns the create intent transition, along with the expected intent
/// object.
pub fn gen_create_intent_from_private_fill_transition(
    expected_state_object: &ExpectedStateObject,
) -> (CreateIntentTransition, DarkpoolStateIntent) {
    let owner = Address::random();

    let intent = gen_random_intent(owner);

    let mut wrapped_intent = DarkpoolStateIntent::new(
        intent,
        expected_state_object.share_stream_seed,
        expected_state_object.recovery_stream_seed,
    );

    // We progress the balance's recovery stream to represent the computation of the
    // 0th recovery ID
    wrapped_intent.recovery_stream.advance_by(1);

    let intent_creation_data =
        IntentCreationData::RenegadeSettledPrivateFill(wrapped_intent.public_share());

    let transition = CreateIntentTransition {
        recovery_id: expected_state_object.recovery_id,
        block_number: 0,
        intent_creation_data,
    };

    (transition, wrapped_intent)
}

/// Generate the state transition which should result in the given
/// expected state object being indexed as a new intent resulting from a
/// public-fill match settlement.
///
/// Returns the create intent transition, along with the expected intent
/// object.
pub fn gen_create_intent_from_public_fill_transition(
    expected_state_object: &ExpectedStateObject,
) -> (CreateIntentTransition, DarkpoolStateIntent) {
    let owner = Address::random();

    let intent = gen_random_intent(owner);

    let mut initial_wrapped_intent = DarkpoolStateIntent::new(
        intent,
        expected_state_object.share_stream_seed,
        expected_state_object.recovery_stream_seed,
    );

    // We progress the balance's recovery stream to represent the computation of the
    // 0th recovery ID
    initial_wrapped_intent.recovery_stream.advance_by(1);

    // Create a dummy settlement obligation with a random match amount
    let settlement_obligation =
        gen_random_settlement_obligation(initial_wrapped_intent.inner.amount_in);

    let mut wrapped_intent = initial_wrapped_intent.clone();
    wrapped_intent.apply_settlement_obligation(&settlement_obligation);

    let intent_creation_data = IntentCreationData::PublicFill {
        pre_match_full_intent_share: initial_wrapped_intent.public_share(),
        settlement_obligation,
    };

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
fn update_balance_amount(balance: &mut DarkpoolStateBalance, new_amount: Amount) -> Scalar {
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

/// Update the protocol fee in a balance.
///
/// Returns the public share of the new protocol fee.
fn update_balance_protocol_fee(
    balance: &mut DarkpoolStateBalance,
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
    balance: &mut DarkpoolStateBalance,
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

/// Generate the state transition which should result in the given
/// balance being updated with a deposit.
///
/// Returns the deposit transition, along with the updated balance.
pub fn gen_deposit_transition(
    initial_balance: &DarkpoolStateBalance,
) -> (DepositTransition, DarkpoolStateBalance) {
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
    initial_balance: &DarkpoolStateBalance,
) -> (WithdrawTransition, DarkpoolStateBalance) {
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
    initial_balance: &DarkpoolStateBalance,
) -> (PayProtocolFeeTransition, DarkpoolStateBalance) {
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
    initial_balance: &DarkpoolStateBalance,
) -> (PayRelayerFeeTransition, DarkpoolStateBalance) {
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
/// balance being updated with a private-fill match settlement.
///
/// Returns the match settlement transition, along with the updated balance.
pub fn gen_settle_private_fill_into_balance_transition(
    initial_balance: &DarkpoolStateBalance,
) -> (SettleMatchIntoBalanceTransition, DarkpoolStateBalance) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Create a dummy settlement obligation with a random match amount
    let settlement_obligation = gen_random_settlement_obligation(initial_balance.inner.amount);

    // For simplicity, we model the balance as an input balance. There is no
    // difference in the state applicator logic between private-fill settlement
    // into an input vs output balance.
    updated_balance.apply_obligation_in_balance(&settlement_obligation);
    let post_match_balance_share = updated_balance.reencrypt_post_match_share();

    updated_balance.recovery_stream.advance_by(1);

    let balance_settlement_data = BalanceSettlementData::PrivateFill(post_match_balance_share);

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoBalanceTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        balance_settlement_data,
    };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// input balance being updated with a the first fill of a public-fill match
/// settlement.
///
/// Returns the match settlement transition, along with the updated balance.
pub fn gen_settle_public_first_fill_into_input_balance_transition(
    initial_balance: &DarkpoolStateBalance,
) -> (SettleMatchIntoBalanceTransition, DarkpoolStateBalance) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Create a dummy settlement obligation with a random match amount
    let settlement_obligation = gen_random_settlement_obligation(initial_balance.inner.amount);

    // TODO: Authority handling has changed from Address to SchnorrPublicKey.
    // Using a default scalar for now until the authority encryption approach is
    // updated.
    let new_one_time_authority_share = Scalar::default();

    updated_balance.reencrypt_post_match_share();
    updated_balance.apply_obligation_in_balance(&settlement_obligation);

    updated_balance.recovery_stream.advance_by(1);

    let balance_settlement_data = BalanceSettlementData::PublicFirstFillInputBalance {
        settlement_obligation,
        new_one_time_authority_share,
    };

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoBalanceTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        balance_settlement_data,
    };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// input balance being updated with a public-fill match settlement.
///
/// Returns the match settlement transition, along with the updated balance.
pub fn gen_settle_public_fill_into_input_balance_transition(
    initial_balance: &DarkpoolStateBalance,
) -> (SettleMatchIntoBalanceTransition, DarkpoolStateBalance) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Create a dummy settlement obligation with a random match amount
    let settlement_obligation = gen_random_settlement_obligation(initial_balance.inner.amount);

    updated_balance.reencrypt_post_match_share();
    updated_balance.apply_obligation_in_balance(&settlement_obligation);

    updated_balance.recovery_stream.advance_by(1);

    let balance_settlement_data =
        BalanceSettlementData::PublicFillInputBalance { settlement_obligation };

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoBalanceTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        balance_settlement_data,
    };

    (transition, updated_balance)
}

/// Generate the state transition which should result in the given
/// output balance being updated with a public-fill match settlement.
///
/// Returns the match settlement transition, along with the updated balance.
pub fn gen_settle_public_fill_into_output_balance_transition(
    initial_balance: &DarkpoolStateBalance,
) -> (SettleMatchIntoBalanceTransition, DarkpoolStateBalance) {
    let spent_nullifier = initial_balance.compute_nullifier();

    let mut updated_balance = initial_balance.clone();

    // Create a dummy settlement obligation with a random match amount
    let settlement_obligation = gen_random_settlement_obligation(initial_balance.inner.amount);

    let relayer_fee_rate = relayer_fee();
    let protocol_fee_rate = protocol_fee();

    let relayer_fee =
        scalar_to_u128(&relayer_fee_rate.floor_mul_int(settlement_obligation.amount_out));

    let protocol_fee =
        scalar_to_u128(&protocol_fee_rate.floor_mul_int(settlement_obligation.amount_out));

    let fee_take = FeeTake { relayer_fee, protocol_fee };

    updated_balance.reencrypt_post_match_share();
    updated_balance.apply_obligation_out_balance(&settlement_obligation, &fee_take);

    updated_balance.recovery_stream.advance_by(1);

    let balance_settlement_data = BalanceSettlementData::PublicFillOutputBalance {
        settlement_obligation,
        relayer_fee_rate,
        protocol_fee_rate,
    };

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoBalanceTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        balance_settlement_data,
    };

    (transition, updated_balance)
}

/// Generate a settle public intent transition with an internal match
/// that will create a new public intent (since none exists yet)
pub fn gen_settle_public_intent_transition(owner: Address) -> SettlePublicIntentTransition {
    let intent = gen_random_intent(owner);
    let intent_signature = gen_mock_intent_signature();
    let permit = mock_public_intent_permit();

    let amount_in = thread_rng().gen_range(1..=intent.amount_in);

    // Create dummy hashes, we don't need to actually hash for testing
    let intent_hash = B256::random();
    let tx_hash = TxHash::random();

    let public_intent_settlement_data =
        PublicIntentSettlementData::InternalMatch { intent, intent_signature, permit, amount_in };

    SettlePublicIntentTransition {
        intent_hash,
        tx_hash,
        block_number: 0,
        public_intent_settlement_data,
    }
}

/// Generate a settle public intent transition with an external match
/// that will create a new public intent (since none exists yet)
pub fn gen_settle_public_intent_external_match_transition(
    owner: Address,
) -> SettlePublicIntentTransition {
    let intent = gen_random_intent(owner);
    let intent_signature = gen_mock_intent_signature();
    let permit = mock_public_intent_permit();

    // Generate a random price for the external match
    let price = random_price();

    // Compute the upper bound for external party amount in.
    // External party amount is in terms of output_token (internal party's output).
    // Upper bound is the internal party's remaining input amount converted to
    // output via price.
    let max_external_amount_in = scalar_to_u128(&price.floor_mul_int(intent.amount_in));

    // Generate a random external party amount in, bounded by what the internal
    // party can provide
    let external_party_amount_in = random_amount().min(max_external_amount_in);

    // Create dummy hashes, we don't need to actually hash for testing
    let intent_hash = B256::random();
    let tx_hash = TxHash::random();

    let public_intent_settlement_data = PublicIntentSettlementData::ExternalMatch {
        intent,
        intent_signature,
        permit,
        price,
        external_party_amount_in,
    };

    SettlePublicIntentTransition {
        intent_hash,
        tx_hash,
        block_number: 0,
        public_intent_settlement_data,
    }
}

/// Generate a settle public intent transition with an internal match
/// that will update an existing public intent.
///
/// Returns the settlement transition.
pub fn gen_settle_public_intent_transition_for_existing(
    initial_public_intent: &PublicIntentStateObject,
) -> SettlePublicIntentTransition {
    // Generate a random match amount
    let amount_in = random_amount().min(initial_public_intent.order.intent.inner.amount_in);

    // Get the existing intent, signature, and permit from the public intent
    let intent = initial_public_intent.order.intent.inner.clone();
    let intent_signature = initial_public_intent.intent_signature.clone();
    let permit = initial_public_intent.permit.clone();

    // Create a dummy tx hash for testing
    let tx_hash = TxHash::random();

    let public_intent_settlement_data =
        PublicIntentSettlementData::InternalMatch { intent, intent_signature, permit, amount_in };

    SettlePublicIntentTransition {
        intent_hash: initial_public_intent.intent_hash,
        tx_hash,
        block_number: 0,
        public_intent_settlement_data,
    }
}

/// Generate a settle public intent transition with an external match
/// that will update an existing public intent.
///
/// Returns the settlement transition.
pub fn gen_settle_public_intent_external_match_transition_for_existing(
    initial_public_intent: &PublicIntentStateObject,
) -> SettlePublicIntentTransition {
    // Generate a random price for the external match
    // Price is in terms of internal party's output_token / input_token
    let price = random_price();

    // Compute the upper bound for external party amount in.
    // External party amount is in terms of output_token (internal party's output).
    // Upper bound is the internal party's remaining input amount converted to
    // output via price.
    let max_external_amount_in =
        scalar_to_u128(&price.floor_mul_int(initial_public_intent.order.intent.inner.amount_in));

    // Generate a random external party amount in, bounded by what the internal
    // party can provide
    let external_party_amount_in = random_amount().min(max_external_amount_in);

    // Get the existing intent, signature, and permit from the public intent
    let intent = initial_public_intent.order.intent.inner.clone();
    let intent_signature = initial_public_intent.intent_signature.clone();
    let permit = initial_public_intent.permit.clone();

    // Create a dummy tx hash for testing
    let tx_hash = TxHash::random();

    let public_intent_settlement_data = PublicIntentSettlementData::ExternalMatch {
        intent,
        intent_signature,
        permit,
        price,
        external_party_amount_in,
    };

    SettlePublicIntentTransition {
        intent_hash: initial_public_intent.intent_hash,
        tx_hash,
        block_number: 0,
        public_intent_settlement_data,
    }
}

/// Generate a public intent metadata update message for testing
pub fn gen_public_intent_metadata_update_message(
    owner: Address,
) -> PublicIntentMetadataUpdateMessage {
    let intent = gen_random_intent(owner);
    let intent_signature = gen_mock_intent_signature();
    let permit = mock_public_intent_permit();
    let intent_hash = B256::random();
    let order_id = Uuid::new_v4();
    let matching_pool = "test-pool".to_string();
    let allow_external_matches = true;
    let min_fill_size = random_amount().min(intent.amount_in);

    PublicIntentMetadataUpdateMessage {
        intent_hash,
        intent,
        intent_signature,
        permit,
        order_id,
        matching_pool,
        allow_external_matches,
        min_fill_size,
    }
}

/// Generate a metadata update message for an existing public intent
pub fn gen_public_intent_metadata_update_message_for_existing(
    existing: &PublicIntentStateObject,
) -> PublicIntentMetadataUpdateMessage {
    // Use same intent_hash but different metadata values
    PublicIntentMetadataUpdateMessage {
        intent_hash: existing.intent_hash,
        intent: existing.order.intent.inner.clone(),
        intent_signature: existing.intent_signature.clone(),
        permit: existing.permit.clone(),
        order_id: Uuid::new_v4(),
        matching_pool: "updated-pool".to_string(),
        allow_external_matches: !existing.order.metadata.allow_external_matches,
        min_fill_size: random_amount().min(existing.order.intent.inner.amount_in),
    }
}

/// Generate the state transition which should result in the given
/// intent being updated with a match settlement.
///
/// To generate a transition for
/// an intent being updated from a Renegade-settled public-fill match, use
/// `gen_settle_public_fill_into_intent_transition`.
///
/// Returns the match settlement transition, along with the updated intent.
pub fn gen_settle_match_into_intent_transition(
    initial_intent: &DarkpoolStateIntent,
) -> (SettleMatchIntoIntentTransition, DarkpoolStateIntent) {
    let spent_nullifier = initial_intent.compute_nullifier();

    let mut updated_intent = initial_intent.clone();

    // Create a dummy settlement obligation with a random match amount
    let settlement_obligation = gen_random_settlement_obligation(initial_intent.inner.amount_in);

    updated_intent.apply_settlement_obligation(&settlement_obligation);
    updated_intent.reencrypt_amount_in();
    updated_intent.recovery_stream.advance_by(1);

    let amount_public_share = updated_intent.public_share().amount_in;

    let intent_settlement_data = IntentSettlementData::UpdatedAmountShare(amount_public_share);

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoIntentTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        intent_settlement_data,
    };

    (transition, updated_intent)
}

/// Generate the state transition which should result in the given
/// intent being updated with a Renegade-settled public-fill match settlement.
///
/// Returns the match settlement transition, along with the updated intent.
pub fn gen_settle_public_fill_into_intent_transition(
    initial_intent: &DarkpoolStateIntent,
) -> (SettleMatchIntoIntentTransition, DarkpoolStateIntent) {
    let spent_nullifier = initial_intent.compute_nullifier();

    let mut updated_intent = initial_intent.clone();

    // Create a dummy settlement obligation with a random match amount
    let settlement_obligation = gen_random_settlement_obligation(initial_intent.inner.amount_in);

    updated_intent.apply_settlement_obligation(&settlement_obligation);
    updated_intent.reencrypt_amount_in();
    updated_intent.recovery_stream.advance_by(1);

    let intent_settlement_data = IntentSettlementData::PublicFill { settlement_obligation };

    // Construct the associated match settlement transition
    let transition = SettleMatchIntoIntentTransition {
        nullifier: spent_nullifier,
        block_number: 0,
        intent_settlement_data,
    };

    (transition, updated_intent)
}

/// Generate the state transition which should result in the given
/// order (intent) being cancelled
pub fn gen_cancel_order_transition(initial_intent: &DarkpoolStateIntent) -> CancelOrderTransition {
    let spent_nullifier = initial_intent.compute_nullifier();

    CancelOrderTransition { nullifier: spent_nullifier, block_number: 0 }
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
    expected_balance: &DarkpoolStateBalance,
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
    expected_intent: &DarkpoolStateIntent,
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
    assert_eq!(&indexed_public_intent.order.intent.inner, expected_intent);

    Ok(())
}

/// Validate the metadata fields of a public intent against expected values
pub async fn validate_public_intent_metadata(
    db_client: &DbClient,
    intent_hash: B256,
    expected_order_id: Uuid,
    expected_matching_pool: &str,
    expected_allow_external_matches: bool,
    expected_min_fill_size: Amount,
) -> Result<(), DbError> {
    let mut conn = db_client.get_db_conn().await?;

    let indexed_public_intent = db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

    assert_eq!(indexed_public_intent.order.id, expected_order_id);
    assert_eq!(indexed_public_intent.matching_pool, expected_matching_pool);
    assert_eq!(
        indexed_public_intent.order.metadata.allow_external_matches,
        expected_allow_external_matches
    );
    assert_eq!(indexed_public_intent.order.metadata.min_fill_size, expected_min_fill_size);

    Ok(())
}
