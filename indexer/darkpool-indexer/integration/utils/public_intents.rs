//! Integration testing utilities for managing public intents

use alloy::{
    primitives::{Address, B256, keccak256},
    rpc::types::TransactionReceipt,
    sol_types::SolValue,
};
use darkpool_indexer::{
    api::http::handlers::get_all_active_user_state_objects,
    indexer::event_indexing::types::settlement_bundle::NATIVELY_SETTLED_PUBLIC_INTENT,
};
use darkpool_indexer_api::types::http::{ApiPublicIntent, ApiStateObject};
use eyre::Result;
use renegade_circuit_types::{intent::Intent, settlement_obligation::SettlementObligation};
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        self, FeeRate, PublicIntentAuthBundle, PublicIntentPermit, PublicIntentPublicBalanceBundle,
        SettlementBundle,
    },
    auth_helpers::sign_with_nonce,
};

use crate::{
    test_args::TestArgs,
    utils::{
        intents::{
            build_public_obligation_bundle, create_intents_and_obligations, settlement_relayer_fee,
            split_obligation,
        },
        transactions::wait_for_tx_success,
    },
};

// -------------------------------
// | Ring 0 Intents / Settlement |
// -------------------------------

/// Submit a settlement between two ring 0 intents which both receive their
/// first fill.
///
/// Returns the transaction receipt, party 0's new intent hash, and both
/// subsequent fill obligations
pub async fn submit_ring0_first_fill(
    args: &TestArgs,
) -> Result<(TransactionReceipt, B256, SettlementObligation, SettlementObligation)> {
    // Build the crossing intents & obligations
    let (intent0, intent1, obligation0, obligation1) = create_intents_and_obligations(args)?;

    // Split the obligations in 2 to allow for 2 fills
    let (first_obligation0, second_obligation0) = split_obligation(&obligation0);
    let (first_obligation1, second_obligation1) = split_obligation(&obligation1);

    let (settlement_bundle0, intent_hash) = build_ring0_settlement_bundle_first_fill(
        args,
        true, // is_party0
        &intent0,
        &first_obligation0,
    )?;

    let (settlement_bundle1, _) = build_ring0_settlement_bundle_first_fill(
        args,
        false, // is_party0
        &intent1,
        &first_obligation1,
    )?;

    let obligation_bundle = build_public_obligation_bundle(&first_obligation0, &first_obligation1);

    let darkpool = args.darkpool_instance();
    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);

    let receipt = wait_for_tx_success(call).await?;

    Ok((receipt, intent_hash, second_obligation0, second_obligation1))
}

/// Build a settlement bundle for the first fill of a ring 0 intent.
///
/// Returns the settlement bundle alongside the intent hash.
fn build_ring0_settlement_bundle_first_fill(
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

    let intent_signature = sign_with_nonce(intent_hash.as_slice(), &owner)?;

    // Generate executor signature
    let relayer_fee_rate =
        FeeRate { rate: settlement_relayer_fee().into(), recipient: Address::random() };

    let obligation: IDarkpoolV2::SettlementObligation = circuit_obligation.clone().into();
    let executor_digest = keccak256((relayer_fee_rate.clone(), obligation).abi_encode());

    let executor_signature = sign_with_nonce(executor_digest.as_slice(), &executor)?;

    let auth = PublicIntentAuthBundle {
        permit,
        intentSignature: intent_signature,
        executorSignature: executor_signature,
    };

    let bundle_data = PublicIntentPublicBalanceBundle { auth, relayerFeeRate: relayer_fee_rate };

    let bundle = SettlementBundle {
        // Even though this is the first fill, the contracts don't expect this field to be set
        isFirstFill: false,
        bundleType: NATIVELY_SETTLED_PUBLIC_INTENT,
        data: bundle_data.abi_encode().into(),
    };

    Ok((bundle, intent_hash))
}

// ----------------
// | Misc Helpers |
// ----------------

/// Get the first public intent state object for the first test account
pub async fn get_party0_first_public_intent(args: &TestArgs) -> Result<ApiPublicIntent> {
    let state_objects =
        get_all_active_user_state_objects(args.party0_account_id(), args.db_client()).await?;

    state_objects
        .into_iter()
        .find_map(|state_object| match state_object {
            ApiStateObject::PublicIntent(public_intent) => Some(public_intent),
            _ => None,
        })
        .ok_or(eyre::eyre!("Public intent not found"))
}
