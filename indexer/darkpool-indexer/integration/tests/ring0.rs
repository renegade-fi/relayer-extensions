//! Tests the indexing of ring 0 (natively-settled, public-intent) match
//! settlements

use std::time::Duration;

use alloy::{
    primitives::{Address, B256, keccak256},
    rpc::types::TransactionReceipt,
    sol_types::SolValue,
};
use eyre::Result;
use renegade_darkpool_types::{intent::Intent, settlement_obligation::SettlementObligation};
use renegade_solidity_abi::v2::calldata_bundles::NATIVE_SETTLED_PUBLIC_INTENT_BUNDLE_TYPE;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        self, FeeRate, ObligationBundle, PublicIntentAuthBundle, PublicIntentPermit,
        PublicIntentPublicBalanceBundle, SettlementBundle,
    },
    relayer_types::u128_to_u256,
};
use test_helpers::assert_eq_result;

use crate::{
    indexer_integration_test,
    test_args::TestArgs,
    utils::{
        test_data::{create_intents_and_obligations, settlement_relayer_fee, split_obligation},
        transactions::wait_for_tx_success,
    },
};

// ---------
// | Tests |
// ---------

/// Test the indexing of the settlement of the first fill of a ring 0 intent
async fn test_ring0_first_fill(args: TestArgs) -> Result<()> {
    let (_, intent_hash, _, _, _, _) = submit_ring0_first_fill(&args).await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the new public intent from the indexer
    let indexed_public_intent = args.get_public_intent_by_hash(intent_hash).await?;

    let indexed_remaining_amount = u128_to_u256(indexed_public_intent.order.intent.inner.amount_in);
    let onchain_remaining_amount = args
        .darkpool_instance()
        .openPublicIntents(indexed_public_intent.intent_hash)
        .call()
        .await?;

    assert_eq_result!(indexed_remaining_amount, onchain_remaining_amount)
}
indexer_integration_test!(test_ring0_first_fill);

/// Test the indexing of the settlement of a subsequent fill of a ring 0 intent
async fn test_ring0_subsequent_fill(args: TestArgs) -> Result<()> {
    let (_, intent_hash, intent0, intent1, second_obligation0, second_obligation1) =
        submit_ring0_first_fill(&args).await?;

    submit_ring0_subsequent_fill(
        &args,
        &intent0,
        &intent1,
        &second_obligation0,
        &second_obligation1,
    )
    .await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the public intent from the indexer
    let indexed_public_intent = args.get_public_intent_by_hash(intent_hash).await?;

    let indexed_remaining_amount = u128_to_u256(indexed_public_intent.order.intent.inner.amount_in);
    let onchain_remaining_amount = args
        .darkpool_instance()
        .openPublicIntents(indexed_public_intent.intent_hash)
        .call()
        .await?;

    assert_eq_result!(indexed_remaining_amount, onchain_remaining_amount)
}
indexer_integration_test!(test_ring0_subsequent_fill);

// -----------
// | Helpers |
// -----------

/// Submit a settlement between two ring 0 intents which both receive their
/// first fill.
///
/// Returns the transaction receipt, party 0's new intent hash, both intents,
/// and both subsequent fill obligations
async fn submit_ring0_first_fill(
    args: &TestArgs,
) -> Result<(TransactionReceipt, B256, Intent, Intent, SettlementObligation, SettlementObligation)>
{
    // Build the crossing intents & obligations
    let (intent0, intent1, obligation0, obligation1) = create_intents_and_obligations(args)?;

    // Split the obligations in 2 to allow for 2 fills
    let (first_obligation0, second_obligation0) = split_obligation(&obligation0);
    let (first_obligation1, second_obligation1) = split_obligation(&obligation1);

    let (settlement_bundle0, intent_hash) = build_ring0_settlement_bundle(
        args,
        true, // is_party0
        &intent0,
        &first_obligation0,
    )
    .await?;

    let (settlement_bundle1, _) = build_ring0_settlement_bundle(
        args,
        false, // is_party0
        &intent1,
        &first_obligation1,
    )
    .await?;

    let obligation_bundle = ObligationBundle::new_public(
        first_obligation0.clone().into(),
        first_obligation1.clone().into(),
    );

    let darkpool = args.darkpool_instance();
    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);

    let receipt = wait_for_tx_success(call).await?;

    Ok((receipt, intent_hash, intent0, intent1, second_obligation0, second_obligation1))
}

/// Submit the settlement of a subsequent fill on the 2 given intents,
/// represented by the given 2 settlement obligations.
///
/// Returns the transaction receipt.
async fn submit_ring0_subsequent_fill(
    args: &TestArgs,
    original_intent0: &Intent,
    original_intent1: &Intent,
    second_obligation0: &SettlementObligation,
    second_obligation1: &SettlementObligation,
) -> Result<TransactionReceipt> {
    let (settlement_bundle0, _) = build_ring0_settlement_bundle(
        args,
        true, // is_party0
        original_intent0,
        second_obligation0,
    )
    .await?;

    let (settlement_bundle1, _) = build_ring0_settlement_bundle(
        args,
        false, // is_party0
        original_intent1,
        second_obligation1,
    )
    .await?;

    let obligation_bundle = ObligationBundle::new_public(
        second_obligation0.clone().into(),
        second_obligation1.clone().into(),
    );

    let darkpool = args.darkpool_instance();
    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);

    let receipt = wait_for_tx_success(call).await?;

    Ok(receipt)
}

/// Build a settlement bundle a ring 0 intent.
///
/// Returns the settlement bundle alongside the intent hash.
pub async fn build_ring0_settlement_bundle(
    args: &TestArgs,
    is_party0: bool,
    circuit_intent: &Intent,
    circuit_obligation: &SettlementObligation,
) -> Result<(SettlementBundle, B256)> {
    // Construct the intent permit
    let intent: IDarkpoolV2::Intent = circuit_intent.clone().into();
    // We'll always execute through party 0
    let executor = args.party0_signer();

    let permit = PublicIntentPermit { intent, executor: executor.address() };

    // Generate intent signature
    let intent_hash = keccak256(permit.abi_encode());
    let owner = if is_party0 { args.party0_signer() } else { args.party1_signer() };

    let chain_id = args.chain_id().await?;
    let intent_signature = permit.sign(chain_id, &owner)?;

    // Generate executor signature
    let relayer_fee_rate =
        FeeRate { rate: settlement_relayer_fee().into(), recipient: Address::random() };

    let obligation: IDarkpoolV2::SettlementObligation = circuit_obligation.clone().into();
    let executor_signature =
        obligation.create_executor_signature(&relayer_fee_rate, chain_id, &executor)?;

    let auth = PublicIntentAuthBundle {
        intentPermit: permit,
        intentSignature: intent_signature,
        executorSignature: executor_signature,
        allowancePermit: Default::default(),
    };

    let bundle_data = PublicIntentPublicBalanceBundle { auth, relayerFeeRate: relayer_fee_rate };

    let bundle = SettlementBundle {
        // The contracts never expect this field to be set
        isFirstFill: false,
        bundleType: NATIVE_SETTLED_PUBLIC_INTENT_BUNDLE_TYPE,
        data: bundle_data.abi_encode().into(),
    };

    Ok((bundle, intent_hash))
}
