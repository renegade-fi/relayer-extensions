//! Tests the indexing of a `depositNewBalance` contract call

use std::time::Duration;

use eyre::Result;
use test_helpers::assert_eq_result;

use crate::{
    indexer_integration_test,
    test_args::TestArgs,
    utils::{
        balance::{deposit_new_balance, get_party0_first_balance, random_deposit},
        transactions::get_first_merkle_insertion,
    },
};

/// Test the indexing of a `depositNewBalance` call
async fn test_deposit_new_balance(mut args: TestArgs) -> Result<()> {
    let deposit = random_deposit(&args)?;
    let (receipt, _balance, recovery_id) = deposit_new_balance(&mut args, &deposit).await?;

    // TEMP: Bypass the chain event listener & enqueue messages directly until event
    // emission is implemented in the contracts
    args.send_recovery_id_registration_message(recovery_id, receipt.transaction_hash).await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the new balance from the indexer. We simply use the first balance state
    // object found for the account, as there should only be one.
    let balance = get_party0_first_balance(&args).await?;

    // Assert that the indexed balance's commitment is included onchain in the
    // Merkle tree
    let indexed_commitment = balance.compute_commitment();
    let inserted_commitment = get_first_merkle_insertion(&receipt).await?;

    assert_eq_result!(indexed_commitment, inserted_commitment)
}
indexer_integration_test!(test_deposit_new_balance);
