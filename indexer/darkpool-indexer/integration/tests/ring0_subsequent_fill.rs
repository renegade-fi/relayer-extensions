//! Tests the indexing of the settlement of a subsequent fill of a ring 0
//! (natively-settled, public-intent) intent

use std::time::Duration;

use eyre::Result;
use renegade_solidity_abi::v2::relayer_types::u128_to_u256;
use test_helpers::assert_eq_result;

use crate::{
    indexer_integration_test,
    test_args::TestArgs,
    utils::public_intents::{
        get_party0_first_public_intent, submit_ring0_first_fill, submit_ring0_subsequent_fill,
    },
};

/// Test the indexing of the settlement of a subsequent fill of a ring 0 intent
async fn test_ring0_subsequent_fill(args: TestArgs) -> Result<()> {
    let (initial_receipt, intent_hash, intent0, intent1, second_obligation0, second_obligation1) =
        submit_ring0_first_fill(&args).await?;

    // TEMP: Bypass the chain event listener & enqueue messages directly until event
    // emission is implemented in the contracts
    args.send_public_intent_creation_message(intent_hash, initial_receipt.transaction_hash).await?;

    let receipt = submit_ring0_subsequent_fill(
        &args,
        &intent0,
        &intent1,
        &second_obligation0,
        &second_obligation1,
    )
    .await?;

    args.send_public_intent_update_message(
        intent_hash,
        1, // version
        receipt.transaction_hash,
    )
    .await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the new public intent from the indexer. We simply use the first public
    // intent found for the account, as there should only be one.
    let public_intent = get_party0_first_public_intent(&args).await?;

    let indexed_remaining_amount = u128_to_u256(public_intent.intent.amount_in);
    let onchain_remaining_amount =
        args.darkpool_instance().openPublicIntents(public_intent.intent_hash).call().await?;

    assert_eq_result!(indexed_remaining_amount, onchain_remaining_amount)
}
indexer_integration_test!(test_ring0_subsequent_fill);
