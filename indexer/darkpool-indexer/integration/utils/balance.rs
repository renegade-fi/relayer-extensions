//! Utilities for managing balances in integration tests

use alloy::{
    primitives::Address, rpc::types::TransactionReceipt, signers::local::PrivateKeySigner,
};
use darkpool_indexer::api::http::handlers::get_all_active_user_state_objects;
use darkpool_indexer_api::types::http::ApiStateObject;
use eyre::Result;
use renegade_circuit_types::{
    balance::{Balance, DarkpoolStateBalance},
    state_wrapper::StateWrapper,
};
use renegade_circuits::{
    singleprover_prove,
    zk_circuits::valid_balance_create::{
        ValidBalanceCreate, ValidBalanceCreateStatement, ValidBalanceCreateWitness,
    },
};
use renegade_constants::Scalar;
use renegade_crypto::fields::{scalar_to_u256, u256_to_scalar};
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{Deposit, DepositAuth, NewBalanceDepositProofBundle},
    relayer_types::u256_to_u128,
    transfer_auth::deposit::create_deposit_permit,
};

use crate::{
    test_args::TestArgs,
    utils::{random_amount_u256, transactions::wait_for_tx_success},
};

/// Helper to create a new balance in the darkpool from the given deposit.
///
/// Assumes that the signer has already been funded with the deposit amount
/// and that the Permit2 contract has been approved to spend the tokens.
///
/// Returns the transaction receipt, new balance state object, and its first
/// recovery ID.
pub async fn deposit_new_balance(
    args: &mut TestArgs,
    deposit: &Deposit,
) -> Result<(TransactionReceipt, DarkpoolStateBalance, Scalar)> {
    // Build calldata for the balance creation
    let (witness, bundle) = gen_new_balance_deposit_proof_bundle(args, deposit)?;
    let commitment = u256_to_scalar(&bundle.statement.newBalanceCommitment);
    let signer = args.party0_signer();
    let deposit_auth = build_deposit_permit(args, commitment, deposit, &signer).await?;

    // Send the txn
    let darkpool = args.darkpool_instance();
    let call = darkpool.depositNewBalance(deposit_auth, bundle.clone());

    let receipt = wait_for_tx_success(call).await?;

    // Build the post-txn balance
    let mut balance = DarkpoolStateBalance::new(
        witness.balance,
        witness.initial_share_stream.seed,
        witness.initial_recovery_stream.seed,
    );

    // Simulate the recovery ID computation that happens in the circuit
    let recovery_id = balance.compute_recovery_id();
    Ok((receipt, balance, recovery_id))
}

/// Generate a proof bundle for a new balance deposit, returning it alongside
/// the associated witness
pub fn gen_new_balance_deposit_proof_bundle(
    args: &mut TestArgs,
    deposit: &Deposit,
) -> Result<(ValidBalanceCreateWitness, NewBalanceDepositProofBundle)> {
    let (witness, statement) = build_new_balance_deposit_witness_statement(args, deposit)?;

    let proof = singleprover_prove::<ValidBalanceCreate>(witness.clone(), statement.clone())?;
    let bundle = NewBalanceDepositProofBundle::new(statement, proof);

    Ok((witness, bundle))
}

/// Build a witness and statement for the new balance deposit
fn build_new_balance_deposit_witness_statement(
    args: &mut TestArgs,
    deposit: &Deposit,
) -> Result<(ValidBalanceCreateWitness, ValidBalanceCreateStatement)> {
    // Build a state object
    let relayer_fee_recipient = Address::random();
    let amount_u128 = u256_to_u128(deposit.amount);
    let balance = Balance::new(
        deposit.token,
        args.party0_address(),
        relayer_fee_recipient,
        args.party0_address(),
    )
    .with_amount(amount_u128);

    // Sample stream seeds
    let share_stream = args.next_party0_share_stream();
    let recovery_stream = args.next_party0_recovery_stream();

    // Encrypt the balance
    let mut initial_state =
        StateWrapper::new(balance.clone(), share_stream.seed, recovery_stream.seed);

    let balance_public_share = initial_state.public_share();
    let recovery_id = initial_state.compute_recovery_id();
    let balance_commitment = initial_state.compute_commitment();

    let witness = ValidBalanceCreateWitness {
        balance,
        initial_share_stream: share_stream,
        initial_recovery_stream: recovery_stream,
    };
    let statement = ValidBalanceCreateStatement {
        deposit: deposit.clone().into(),
        new_balance_share: balance_public_share,
        recovery_id,
        balance_commitment,
    };

    Ok((witness, statement))
}

/// Build a Permit2 signature for the given deposit
pub async fn build_deposit_permit(
    args: &TestArgs,
    new_balance_commitment: Scalar,
    deposit: &Deposit,
    signer: &PrivateKeySigner,
) -> Result<DepositAuth> {
    let commitment = scalar_to_u256(&new_balance_commitment);

    let chain_id = args.chain_id().await?;
    let darkpool = args.darkpool_address();
    let permit2 = args.permit2_address()?;

    // Call create_deposit_permit with all required parameters
    let (witness, signature) =
        create_deposit_permit(commitment, deposit.clone(), chain_id, darkpool, permit2, signer)?;

    let sig_bytes = signature.as_bytes().to_vec();
    Ok(DepositAuth {
        permit2Nonce: witness.nonce,
        permit2Deadline: witness.deadline,
        permit2Signature: sig_bytes.into(),
    })
}

/// Generate a deposit for the first test account w/ a random amount
pub fn random_deposit(args: &TestArgs) -> Result<Deposit> {
    Ok(Deposit {
        from: args.party0_address(),
        token: args.base_token_address()?,
        amount: random_amount_u256(),
    })
}

/// Get the first balance state object for the first test account
pub async fn get_party0_first_balance(args: &TestArgs) -> Result<DarkpoolStateBalance> {
    let state_objects =
        get_all_active_user_state_objects(args.party0_account_id(), args.db_client()).await?;

    state_objects
        .into_iter()
        .find_map(|state_object| match state_object {
            ApiStateObject::Balance(balance) => Some(balance.balance),
            _ => None,
        })
        .ok_or(eyre::eyre!("Balance not found"))
}
