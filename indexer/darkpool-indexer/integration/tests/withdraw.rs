//! Tests the indexing of a `withdraw` contract call

use std::time::Duration;

use alloy::{primitives::U256, rpc::types::TransactionReceipt};
use eyre::Result;
use rand::{Rng, thread_rng};
use renegade_circuit_types::{
    Amount, balance::DarkpoolStateBalance, withdrawal::Withdrawal as CircuitWithdrawal,
};
use renegade_circuits::{
    singleprover_prove,
    zk_circuits::valid_withdrawal::{
        SizedValidWithdrawal, SizedValidWithdrawalWitness, ValidWithdrawalStatement,
        ValidWithdrawalWitness,
    },
};
use renegade_common::types::merkle::MerkleAuthenticationPath;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{Withdrawal, WithdrawalProofBundle},
    relayer_types::u256_to_u128,
    transfer_auth::withdrawal::create_withdrawal_auth,
};
use test_helpers::assert_eq_result;

use crate::{
    indexer_integration_test,
    test_args::TestArgs,
    tests::deposit::submit_deposit_new_balance,
    utils::{
        merkle::{fetch_merkle_opening, find_commitment},
        test_data::random_deposit,
        transactions::wait_for_tx_success,
    },
};

// ---------
// | Tests |
// ---------

/// Test the indexing of a `withdraw` call
async fn test_withdraw(mut args: TestArgs) -> Result<()> {
    // Deposit the initial balance
    let initial_deposit = random_deposit(&args)?;
    let (initial_receipt, mut initial_balance, recovery_id) =
        submit_deposit_new_balance(&mut args, &initial_deposit).await?;

    // TEMP: Bypass the chain event listener & enqueue messages directly until event
    // emission is implemented in the contracts
    args.send_recovery_id_registration_message(recovery_id, initial_receipt.transaction_hash)
        .await?;

    // Submit the subsequent withdrawal
    let receipt = submit_withdrawal(&args, &initial_balance).await?;

    let spent_nullifier = initial_balance.compute_nullifier();
    args.send_nullifier_spend_message(spent_nullifier, receipt.transaction_hash).await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the balance from the indexer.
    // We advance the balance's recovery stream to compute the correct nullifier for
    // the lookup.
    initial_balance.recovery_stream.advance_by(1);
    let indexed_balance =
        args.get_balance_by_nullifier(initial_balance.compute_nullifier()).await?;

    // Assert that the indexed balance's commitment is included onchain in the
    // Merkle tree
    let indexed_commitment = indexed_balance.balance.compute_commitment();
    let commitment_found =
        find_commitment(indexed_commitment, &args.darkpool_instance()).await.is_ok();

    assert_eq_result!(commitment_found, true)
}
indexer_integration_test!(test_withdraw);

// -----------
// | Helpers |
// -----------

/// Submit a transaction which withdraws from an existing balance.
///
/// Returns the transaction receipt.
async fn submit_withdrawal(
    args: &TestArgs,
    initial_balance: &DarkpoolStateBalance,
) -> Result<TransactionReceipt> {
    let initial_commitment = initial_balance.compute_commitment();
    let merkle_path = fetch_merkle_opening(initial_commitment, &args.darkpool_instance()).await?;

    let withdrawal = random_withdrawal(initial_balance.inner.amount, args)?;
    let proof_bundle = gen_withdrawal_proof_bundle(&withdrawal, initial_balance, &merkle_path)?;
    let withdrawal_auth =
        create_withdrawal_auth(proof_bundle.statement.newBalanceCommitment, &args.party0_signer())?;

    let darkpool = args.darkpool_instance();
    let call = darkpool.withdraw(withdrawal_auth, proof_bundle);
    let receipt = wait_for_tx_success(call).await?;

    Ok(receipt)
}

/// Generate a proof bundle for a withdrawal
fn gen_withdrawal_proof_bundle(
    withdrawal: &Withdrawal,
    balance: &DarkpoolStateBalance,
    opening: &MerkleAuthenticationPath,
) -> Result<WithdrawalProofBundle> {
    let (witness, statement) = build_withdrawal_witness_statement(withdrawal, balance, opening)?;

    let proof = singleprover_prove::<SizedValidWithdrawal>(&witness, &statement)?;

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
fn random_withdrawal(max_amount: Amount, args: &TestArgs) -> Result<Withdrawal> {
    let amount = thread_rng().gen_range(0..max_amount);
    Ok(Withdrawal {
        to: args.party0_address(),
        token: args.base_token_address()?,
        amount: U256::from(amount),
    })
}
