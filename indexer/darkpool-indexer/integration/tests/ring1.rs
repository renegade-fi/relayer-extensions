//! Tests the indexing of ring 1 (natively-settled, private-intent) match
//! settlements

use std::time::Duration;

use alloy::{primitives::U256, rpc::types::TransactionReceipt, signers::local::PrivateKeySigner};
use eyre::Result;
use renegade_circuit_types::{
    Commitment, PlonkLinkProof, PlonkProof, ProofLinkingHint,
    intent::{DarkpoolStateIntent, Intent},
    settlement_obligation::SettlementObligation,
};
use renegade_circuits::{
    singleprover_prove_with_hint,
    test_helpers::random_scalar,
    zk_circuits::{
        proof_linking::intent_only::link_sized_intent_only_settlement,
        settlement::intent_only_public_settlement::{
            self, IntentOnlyPublicSettlementStatement, SizedIntentOnlyPublicSettlementCircuit,
        },
        validity_proofs::{
            intent_only::{self, IntentOnlyValidityStatement, SizedIntentOnlyValidityCircuit},
            intent_only_first_fill::{
                IntentOnlyFirstFillValidityCircuit, IntentOnlyFirstFillValidityStatement,
                IntentOnlyFirstFillValidityWitness,
            },
        },
    },
};
use renegade_common::types::merkle::MerkleAuthenticationPath;
use renegade_constants::MERKLE_HEIGHT;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ObligationBundle, PrivateIntentAuthBundle, PrivateIntentAuthBundleFirstFill,
        SettlementBundle,
    },
    auth_helpers::sign_with_nonce,
};
use test_helpers::assert_eq_result;

use crate::{
    indexer_integration_test,
    test_args::TestArgs,
    tests::ring0::build_ring0_settlement_bundle,
    utils::{
        merkle::{find_commitment, parse_merkle_opening_from_receipt},
        test_data::{create_intents_and_obligations, settlement_relayer_fee, split_obligation},
        transactions::wait_for_tx_success,
    },
};

// ---------
// | Tests |
// ---------

/// Test the indexing of the settlement of the first fill of a ring 1 intent
async fn test_ring1_first_fill(mut args: TestArgs) -> Result<()> {
    let (_, state_intent0, _, _, _) = submit_ring1_first_fill(&mut args).await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the indexed intent from the indexer
    let indexed_intent = args.get_intent_by_nullifier(state_intent0.compute_nullifier()).await?;

    // Assert that the indexed balance's commitment is included onchain in the
    // Merkle tree
    let indexed_commitment = indexed_intent.intent.compute_commitment();
    let commitment_found =
        find_commitment(indexed_commitment, &args.darkpool_instance()).await.is_ok();

    assert_eq_result!(commitment_found, true)
}
indexer_integration_test!(test_ring1_first_fill);

/// Test the indexing of the settlement of a subsequent fill of a ring 1 intent
async fn test_ring1_subsequent_fill(mut args: TestArgs) -> Result<()> {
    // Submit the first fill of the intent, so that it is created in the indexer
    let (receipt, mut state_intent0, intent1, second_obligation0, second_obligation1) =
        submit_ring1_first_fill(&mut args).await?;

    // Submit the subsequent fill of the intent
    let receipt = submit_ring1_subsequent_fill(
        &args,
        &state_intent0,
        &intent1,
        &second_obligation0,
        &second_obligation1,
        &receipt,
    )
    .await?;

    // TEMP: Bypass the chain event listener & enqueue messages directly until event
    // emission is implemented in the contracts
    let spent_nullifier = state_intent0.compute_nullifier();
    args.send_nullifier_spend_message(spent_nullifier, receipt.transaction_hash).await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch the indexed intent from the indexer.
    // We advance the intent's recovery stream to compute the correct nullifier for
    // the lookup.
    state_intent0.recovery_stream.advance_by(1);
    let indexed_intent = args.get_intent_by_nullifier(state_intent0.compute_nullifier()).await?;

    // Assert that the indexed balance's commitment is included onchain in the
    // Merkle tree
    let indexed_commitment = indexed_intent.intent.compute_commitment();
    let commitment_found =
        find_commitment(indexed_commitment, &args.darkpool_instance()).await.is_ok();

    assert_eq_result!(commitment_found, true)
}
indexer_integration_test!(test_ring1_subsequent_fill);

// -----------
// | Helpers |
// -----------

/// Submit a settlement between two ring 1 intents which both receive their
/// first fill.
///
/// Returns the transaction receipt, party 0's intent state object (after the
/// first fill has been applied), party 1's intent, and both subsequent fill
/// obligations
async fn submit_ring1_first_fill(
    args: &mut TestArgs,
) -> Result<(
    TransactionReceipt,
    DarkpoolStateIntent,
    Intent,
    SettlementObligation,
    SettlementObligation,
)> {
    // Build the crossing intents & obligations
    let (intent0, intent1, obligation0, obligation1) = create_intents_and_obligations(args)?;

    // Split the obligations in 2 to allow for 2 fills
    let (first_obligation0, second_obligation0) = split_obligation(&obligation0);
    let (first_obligation1, second_obligation1) = split_obligation(&obligation1);

    let (mut state_intent0, settlement_bundle0) = build_ring1_settlement_bundle_first_fill(
        args,
        true, // is_party0
        &intent0,
        &first_obligation0,
    )?;

    // We build a ring 0 settlement bundle for party 1 for simplicity - this way, we
    // don't need to find a Merkle opening for party 1's intent on subsequent fills
    let (settlement_bundle1, _) = build_ring0_settlement_bundle(
        args,
        false, // is_party
        &intent1,
        &first_obligation1,
    )?;

    let obligation_bundle = ObligationBundle::new_public(
        first_obligation0.clone().into(),
        first_obligation1.clone().into(),
    );

    let darkpool = args.darkpool_instance();
    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);

    let receipt = wait_for_tx_success(call).await?;

    state_intent0.apply_settlement_obligation(&first_obligation0);

    Ok((receipt, state_intent0, intent1, second_obligation0, second_obligation1))
}

/// Submit the settlement of a subsequent fill on the 2 given intents,
/// represented by the given 2 settlement obligations.
///
/// Returns the transaction receipt
async fn submit_ring1_subsequent_fill(
    args: &TestArgs,
    state_intent0: &DarkpoolStateIntent,
    intent1: &Intent,
    obligation0: &SettlementObligation,
    obligation1: &SettlementObligation,
    receipt: &TransactionReceipt,
) -> Result<TransactionReceipt> {
    let darkpool = args.darkpool_instance();

    let commitment0 = state_intent0.compute_commitment();

    let opening0 = parse_merkle_opening_from_receipt(commitment0, receipt)?;

    let settlement_bundle0 =
        build_ring1_settlement_bundle_subsequent_fill(state_intent0, &opening0, obligation0)?;

    let (settlement_bundle1, _) =
        build_ring0_settlement_bundle(args, false /* is_party0 */, intent1, obligation1)?;

    let obligation_bundle =
        ObligationBundle::new_public(obligation0.clone().into(), obligation1.clone().into());

    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);
    let receipt = wait_for_tx_success(call).await?;

    Ok(receipt)
}

/// Build a settlement bundle for the first fill of a ring 1 intent
fn build_ring1_settlement_bundle_first_fill(
    args: &mut TestArgs,
    is_party0: bool,
    intent: &Intent,
    obligation: &SettlementObligation,
) -> Result<(DarkpoolStateIntent, SettlementBundle)> {
    // Generate proofs
    let (state_intent, validity_statement, validity_proof, validity_link_hint) =
        generate_ring1_first_fill_validity_proof(args, is_party0, intent)?;

    let (settlement_statement, settlement_proof, settlement_link_hint) =
        generate_ring1_settlement_proof(intent, obligation)?;

    let linking_proof = generate_ring1_linking_proof(&validity_link_hint, &settlement_link_hint)?;

    // Build bundles
    let commitment = state_intent.compute_commitment();
    let owner = if is_party0 { args.party0_signer() } else { args.party1_signer() };
    let auth_bundle =
        build_auth_bundle_first_fill(&owner, commitment, &validity_statement, &validity_proof)?;

    let settlement_bundle = SettlementBundle::private_intent_public_balance_first_fill(
        auth_bundle.clone(),
        settlement_statement.clone().into(),
        settlement_proof.clone().into(),
        linking_proof.into(),
    );

    Ok((state_intent, settlement_bundle))
}

/// Generate a linking proof between a ring 1 validity proof and settlement
/// proof
fn generate_ring1_linking_proof(
    validity_link_hint: &ProofLinkingHint,
    settlement_link_hint: &ProofLinkingHint,
) -> Result<PlonkLinkProof> {
    let proof = link_sized_intent_only_settlement(validity_link_hint, settlement_link_hint)?;

    Ok(proof)
}

/// Build a settlement bundle for a subsequent fill of a ring 1 intent
fn build_ring1_settlement_bundle_subsequent_fill(
    intent: &DarkpoolStateIntent,
    opening: &MerkleAuthenticationPath,
    obligation: &SettlementObligation,
) -> Result<SettlementBundle> {
    let (validity_statement, validity_proof, validity_link_hint) =
        generate_ring1_subsequent_fill_validity_proof(intent, opening)?;

    let (settlement_statement, settlement_proof, settlement_link_hint) =
        generate_ring1_settlement_proof(&intent.inner, obligation)?;

    let linking_proof = generate_ring1_linking_proof(&validity_link_hint, &settlement_link_hint)?;

    let auth_bundle = build_auth_bundle_subsequent_fill(&validity_statement, &validity_proof)?;

    Ok(SettlementBundle::private_intent_public_balance(
        auth_bundle.clone(),
        settlement_statement.clone().into(),
        settlement_proof.clone().into(),
        linking_proof.into(),
    ))
}

/// Generate a validity proof for the first fill of a ring 1 intent
fn generate_ring1_first_fill_validity_proof(
    args: &mut TestArgs,
    is_party0: bool,
    intent: &Intent,
) -> Result<(DarkpoolStateIntent, IntentOnlyFirstFillValidityStatement, PlonkProof, ProofLinkingHint)>
{
    // Build the witness and statement
    let (witness, statement, state_intent) =
        create_intent_only_first_fill_validity_witness_statement(args, is_party0, intent);

    // Generate the validity proof
    let (proof, link_hint) =
        singleprover_prove_with_hint::<IntentOnlyFirstFillValidityCircuit>(&witness, &statement)?;

    Ok((state_intent, statement, proof, link_hint))
}

/// Create a witness and statement for the `IntentOnlyFirstFillValidityCircuit`
/// using the given intent
fn create_intent_only_first_fill_validity_witness_statement(
    args: &mut TestArgs,
    is_party0: bool,
    intent: &Intent,
) -> (IntentOnlyFirstFillValidityWitness, IntentOnlyFirstFillValidityStatement, DarkpoolStateIntent)
{
    // Create the witness intent with initial stream states
    let (share_stream_seed, recovery_stream_seed) = if is_party0 {
        (args.next_party0_share_stream().seed, args.next_party0_recovery_stream().seed)
    } else {
        (random_scalar(), random_scalar())
    };

    let initial_intent =
        DarkpoolStateIntent::new(intent.clone(), share_stream_seed, recovery_stream_seed);

    let mut state_intent = initial_intent.clone();
    let recovery_id = state_intent.compute_recovery_id();
    let intent_private_commitment = state_intent.compute_private_commitment();

    // Get shares from the initial (pre-mutation) state
    let private_shares = initial_intent.private_shares();
    let intent_public_share = initial_intent.public_share();

    // Build the witness with the pre-mutation state
    let witness = IntentOnlyFirstFillValidityWitness {
        intent: initial_intent.inner,
        initial_intent_share_stream: initial_intent.share_stream,
        initial_intent_recovery_stream: initial_intent.recovery_stream,
        private_shares,
    };
    let statement = IntentOnlyFirstFillValidityStatement {
        owner: intent.owner,
        intent_private_commitment,
        recovery_id,
        intent_public_share,
    };

    (witness, statement, state_intent)
}

/// Generate a validity proof for the first fill of a ring 1 intent
fn generate_ring1_subsequent_fill_validity_proof(
    intent: &DarkpoolStateIntent,
    merkle_opening: &MerkleAuthenticationPath,
) -> Result<(IntentOnlyValidityStatement, PlonkProof, ProofLinkingHint)> {
    // Generate the witness and statement
    let (mut witness, mut statement) =
        intent_only::test_helpers::create_witness_statement_with_state_intent(intent.clone());

    // Replace the dummy Merkle opening with the real one
    statement.merkle_root = merkle_opening.compute_root();
    witness.old_intent_opening = merkle_opening.clone().into();

    // Prove the circuit
    let (proof, link_hint) =
        singleprover_prove_with_hint::<SizedIntentOnlyValidityCircuit>(&witness, &statement)?;

    Ok((statement, proof, link_hint))
}

/// Generate a settlement proof for a ring 1 intent
fn generate_ring1_settlement_proof(
    intent: &Intent,
    obligation: &SettlementObligation,
) -> Result<(IntentOnlyPublicSettlementStatement, PlonkProof, ProofLinkingHint)> {
    let (witness, mut statement) = intent_only_public_settlement::test_helpers::create_witness_statement_with_intent_and_obligation(intent, obligation);
    statement.relayer_fee = settlement_relayer_fee();

    let (proof, link_hint) = singleprover_prove_with_hint::<SizedIntentOnlyPublicSettlementCircuit>(
        &witness, &statement,
    )?;

    Ok((statement, proof, link_hint))
}

/// Build an auth bundle for an intent
fn build_auth_bundle_first_fill(
    owner: &PrivateKeySigner,
    commitment: Commitment,
    validity_statement: &IntentOnlyFirstFillValidityStatement,
    validity_proof: &PlonkProof,
) -> Result<PrivateIntentAuthBundleFirstFill> {
    let signature = sign_with_nonce(&commitment.to_bytes_be(), owner)?;

    Ok(PrivateIntentAuthBundleFirstFill {
        intentSignature: signature,
        merkleDepth: U256::from(MERKLE_HEIGHT),
        statement: validity_statement.clone().into(),
        validityProof: validity_proof.clone().into(),
    })
}

/// Build an auth bundle for a subsequent fill
fn build_auth_bundle_subsequent_fill(
    validity_statement: &IntentOnlyValidityStatement,
    validity_proof: &PlonkProof,
) -> Result<PrivateIntentAuthBundle> {
    Ok(PrivateIntentAuthBundle {
        merkleDepth: U256::from(MERKLE_HEIGHT),
        statement: validity_statement.clone().into(),
        validityProof: validity_proof.clone().into(),
    })
}
