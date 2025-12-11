//! Integration testing utilities for managing balances

use alloy::{
    primitives::{Address, U256},
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
};
use darkpool_indexer::api::http::handlers::get_all_active_user_state_objects;
use darkpool_indexer_api::types::http::ApiStateObject;
use eyre::Result;
use rand::{Rng, thread_rng};
use renegade_circuit_types::{
    Amount,
    balance::{Balance, DarkpoolStateBalance},
    state_wrapper::StateWrapper,
    withdrawal::Withdrawal as CircuitWithdrawal,
};
use renegade_circuits::{
    singleprover_prove,
    zk_circuits::{
        valid_balance_create::{
            ValidBalanceCreate, ValidBalanceCreateStatement, ValidBalanceCreateWitness,
        },
        valid_deposit::{
            SizedValidDeposit, SizedValidDepositWitness, ValidDepositStatement, ValidDepositWitness,
        },
        valid_withdrawal::{
            SizedValidWithdrawal, SizedValidWithdrawalWitness, ValidWithdrawalStatement,
            ValidWithdrawalWitness,
        },
    },
};
use renegade_common::types::merkle::MerkleAuthenticationPath;
use renegade_constants::Scalar;
use renegade_crypto::fields::{scalar_to_u256, u256_to_scalar};
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        Deposit, DepositAuth, DepositProofBundle, NewBalanceDepositProofBundle, Withdrawal,
        WithdrawalProofBundle,
    },
    relayer_types::u256_to_u128,
    transfer_auth::{deposit::create_deposit_permit, withdrawal::create_withdrawal_auth},
};

use crate::{
    test_args::TestArgs,
    utils::{merkle::fetch_merkle_opening, random_amount_u256, transactions::wait_for_tx_success},
};

// -----------------------
// | Deposit New Balance |
// -----------------------

/// Helper to create a new balance in the darkpool from the given deposit.
///
/// Assumes that the signer has already been funded with the deposit amount
/// and that the Permit2 contract has been approved to spend the tokens.
///
/// Returns the transaction receipt, the new balance state object, and its first
/// recovery ID.
pub async fn submit_deposit_new_balance(
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

// -----------
// | Deposit |
// -----------

/// Submit a transaction which deposits into an existing balance.
///
/// Returns the transaction receipt, and the balance's spent nullifier.
pub async fn submit_deposit(args: &TestArgs, initial_balance: &DarkpoolStateBalance) -> Result<()> {
    let initial_commitment = initial_balance.compute_commitment();
    let merkle_path = fetch_merkle_opening(initial_commitment, &args.darkpool_instance()).await?;

    let second_deposit = random_deposit(args)?;
    let proof_bundle = gen_deposit_proof_bundle(&second_deposit, initial_balance, &merkle_path)?;
    let commitment = u256_to_scalar(&proof_bundle.statement.newBalanceCommitment);
    let deposit_auth =
        build_deposit_permit(args, commitment, &second_deposit, &args.party0_signer()).await?;

    let darkpool = args.darkpool_instance();
    let call = darkpool.deposit(deposit_auth, proof_bundle);
    wait_for_tx_success(call).await?;

    Ok(())
}

/// Create a proof of the deposit
pub fn gen_deposit_proof_bundle(
    deposit: &Deposit,
    balance: &DarkpoolStateBalance,
    opening: &MerkleAuthenticationPath,
) -> Result<DepositProofBundle> {
    let (witness, statement) = build_deposit_witness_statement(deposit, balance, opening)?;

    let proof = singleprover_prove::<SizedValidDeposit>(witness, statement.clone())?;
    let bundle = DepositProofBundle::new(statement, proof);
    Ok(bundle)
}

/// Build a witness statement for the deposit
fn build_deposit_witness_statement(
    deposit: &Deposit,
    balance: &DarkpoolStateBalance,
    opening: &MerkleAuthenticationPath,
) -> Result<(SizedValidDepositWitness, ValidDepositStatement)> {
    let witness = ValidDepositWitness {
        old_balance: balance.clone(),
        old_balance_opening: opening.clone().into(),
    };

    // Build the new balance and re-encrypt the amount field
    let old_balance_nullifier = balance.compute_nullifier();
    let mut new_balance = balance.clone();
    new_balance.inner.amount += u256_to_u128(deposit.amount);

    let new_amount = new_balance.inner.amount;
    let new_public_share = new_balance.stream_cipher_encrypt(&new_amount);
    new_balance.public_share.amount = new_public_share;

    // Compute a recovery ID and new balance commitment
    let recovery_id = new_balance.compute_recovery_id();
    let new_balance_commitment = new_balance.compute_commitment();

    let merkle_root = opening.compute_root();
    let statement = ValidDepositStatement {
        deposit: deposit.clone().into(),
        merkle_root,
        old_balance_nullifier,
        new_balance_commitment,
        recovery_id,
        new_amount_share: new_public_share,
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

// ------------
// | Withdraw |
// ------------

/// Submit a transaction which withdraws from an existing balance.
///
/// Returns the transaction receipt, and the balance's spent nullifier.
pub async fn submit_withdrawal(
    args: &TestArgs,
    initial_balance: &DarkpoolStateBalance,
) -> Result<()> {
    let initial_commitment = initial_balance.compute_commitment();
    let merkle_path = fetch_merkle_opening(initial_commitment, &args.darkpool_instance()).await?;

    let withdrawal = random_withdrawal(initial_balance.inner.amount, args)?;
    let proof_bundle = gen_withdrawal_proof_bundle(&withdrawal, initial_balance, &merkle_path)?;
    let withdrawal_auth =
        create_withdrawal_auth(proof_bundle.statement.newBalanceCommitment, &args.party0_signer())?;

    let darkpool = args.darkpool_instance();
    let call = darkpool.withdraw(withdrawal_auth, proof_bundle);
    wait_for_tx_success(call).await?;

    Ok(())
}

/// Generate a proof bundle for a withdrawal
pub fn gen_withdrawal_proof_bundle(
    withdrawal: &Withdrawal,
    balance: &DarkpoolStateBalance,
    opening: &MerkleAuthenticationPath,
) -> Result<WithdrawalProofBundle> {
    let (witness, statement) = build_withdrawal_witness_statement(withdrawal, balance, opening)?;

    let proof = singleprover_prove::<SizedValidWithdrawal>(witness, statement.clone())?;

    // Create the bundle using the helper
    let bundle = WithdrawalProofBundle::new(statement, proof);
    Ok(bundle)
}

/// Build a witness statement for the withdrawal
fn build_withdrawal_witness_statement(
    withdrawal: &Withdrawal,
    balance: &DarkpoolStateBalance,
    opening: &MerkleAuthenticationPath,
) -> Result<(SizedValidWithdrawalWitness, ValidWithdrawalStatement)> {
    let witness = ValidWithdrawalWitness {
        old_balance: balance.clone(),
        old_balance_opening: opening.clone().into(),
    };

    // Build the new balance and re-encrypt the amount field
    let old_balance_nullifier = balance.compute_nullifier();
    let mut new_balance = balance.clone();
    new_balance.inner.amount -= u256_to_u128(withdrawal.amount);

    let new_amount = new_balance.inner.amount;
    let new_public_share = new_balance.stream_cipher_encrypt(&new_amount);
    new_balance.public_share.amount = new_public_share;

    // Compute a recovery ID and new balance commitment
    let recovery_id = new_balance.compute_recovery_id();
    let new_balance_commitment = new_balance.compute_commitment();

    let merkle_root = opening.compute_root();

    // Convert ABI Withdrawal to circuit Withdrawal
    let circuit_withdrawal = CircuitWithdrawal {
        to: withdrawal.to,
        token: withdrawal.token,
        amount: u256_to_u128(withdrawal.amount),
    };

    let statement = ValidWithdrawalStatement {
        withdrawal: circuit_withdrawal,
        merkle_root,
        old_balance_nullifier,
        new_balance_commitment,
        recovery_id,
        new_amount_share: new_public_share,
    };

    Ok((witness, statement))
}

/// Create a random withdrawal
pub fn random_withdrawal(max_amount: Amount, args: &TestArgs) -> Result<Withdrawal> {
    let mut rng = thread_rng();
    let amount = rng.gen_range(0..max_amount);
    Ok(Withdrawal {
        to: args.party0_address(),
        token: args.base_token_address()?,
        amount: U256::from(amount),
    })
}

// ----------------
// | Misc Helpers |
// ----------------

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
