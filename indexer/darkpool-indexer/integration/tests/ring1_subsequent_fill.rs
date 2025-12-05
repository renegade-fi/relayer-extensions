//! Tests the indexing of the settlement of a subsequent fill of a ring 1
//! (natively-settled, private-intent) intent

use std::time::Duration;

use eyre::Result;
use test_helpers::assert_eq_result;

use crate::{
    indexer_integration_test,
    test_args::TestArgs,
    utils::{
        intents::{get_party0_first_intent, submit_ring1_first_fill, submit_ring1_subsequent_fill},
        merkle::find_commitment,
    },
};

/// Test the indexing of the settlement of a subsequent fill of a ring 1 intent
async fn test_ring1_subsequent_fill(mut args: TestArgs) -> Result<()> {
    // Submit the first fill of the intent, so that it is created in the indexer
    let (receipt, state_intent0, state_intent1, second_obligation0, second_obligation1) =
        submit_ring1_first_fill(&mut args).await?;

    // Submit the subsequent fill of the intent
    submit_ring1_subsequent_fill(
        &args,
        &state_intent0,
        &state_intent1,
        &second_obligation0,
        &second_obligation1,
        &receipt,
    )
    .await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the intent from the indexer. We simply use the first intent state
    // object found for the account, as there should only be one.
    let intent = get_party0_first_intent(&args).await?;

    // Assert that the indexed balance's commitment is included onchain in the
    // Merkle tree
    let indexed_commitment = intent.compute_commitment();
    let commitment_found =
        find_commitment(indexed_commitment, &args.darkpool_instance()).await.is_ok();

    assert_eq_result!(commitment_found, true)
}
indexer_integration_test!(test_ring1_subsequent_fill);
