//! Tests the indexing of ring 2 (Renegade-settled, public-fill) match
//! settlements

use std::time::Duration;

use alloy::{
    primitives::{Address, U256},
    rpc::types::TransactionReceipt,
};
use eyre::Result;
use renegade_circuit_types::{PlonkProof, ProofLinkingHint, schnorr::SchnorrPrivateKey};
use renegade_circuits::{
    singleprover_prove_with_hint,
    zk_circuits::{
        proof_linking::{
            intent_and_balance::link_sized_intent_and_balance_settlement,
            output_balance::link_sized_output_balance_settlement,
        },
        settlement::intent_and_balance_public_settlement::{
            IntentAndBalancePublicSettlementCircuit, IntentAndBalancePublicSettlementStatement,
            IntentAndBalancePublicSettlementWitness,
        },
        validity_proofs::{
            intent_and_balance_first_fill::{
                BALANCE_PARTIAL_COMMITMENT_SIZE, IntentAndBalanceFirstFillValidityStatement,
                IntentAndBalanceFirstFillValidityWitness,
                SizedIntentAndBalanceFirstFillValidityCircuit,
                SizedIntentAndBalanceFirstFillValidityWitness,
            },
            new_output_balance::{
                NEW_BALANCE_PARTIAL_COMMITMENT_SIZE, NewOutputBalanceValidityStatement,
                SizedNewOutputBalanceValidityCircuit, SizedNewOutputBalanceValidityWitness,
            },
        },
    },
};
use renegade_constants::MERKLE_HEIGHT;
use renegade_darkpool_types::{
    balance::{DarkpoolBalance, DarkpoolStateBalance, PostMatchBalanceShare, PreMatchBalanceShare},
    intent::{DarkpoolStateIntent, Intent, PreMatchIntentShare},
    settlement_obligation::SettlementObligation,
};
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, OutputBalanceBundle, RenegadeSettledIntentAuthBundleFirstFill,
    SettlementBundle,
};
use renegade_types_account::MerkleAuthenticationPath;

use crate::{
    indexer_integration_test,
    test_args::TestArgs,
    tests::{deposit::submit_deposit_new_balance, ring0::build_ring0_settlement_bundle},
    utils::{
        assertions::assert_state_object_committed,
        merkle::fetch_merkle_opening,
        test_data::{
            create_intents_and_obligations, random_deposit, settlement_relayer_fee,
            split_obligation,
        },
        transactions::wait_for_tx_success,
    },
};

// ---------
// | Tests |
// ---------

/// Test the indexing of the settlement of the first fill of a ring 2 intent
async fn test_ring2_first_fill(mut args: TestArgs) -> Result<()> {
    // Submit the deposit for the input balance
    let deposit = random_deposit(&args)?;
    let (deposit_receipt, mut input_balance, input_balance_recovery_id) =
        submit_deposit_new_balance(&mut args, &deposit).await?;

    // TEMP: Bypass the chain event listener & enqueue messages directly until event
    // emission is implemented in the contracts
    args.send_recovery_id_registration_message(
        input_balance_recovery_id,
        deposit_receipt.transaction_hash,
    )
    .await?;

    // Submit the settlement of the first fill
    let (receipt, state_intent0, out_balance0) =
        submit_ring2_first_fill(&mut args, &mut input_balance).await?;

    // TEMP: Bypass the chain event listener & enqueue messages directly until event
    // emission is implemented in the contracts.
    // We roll back the input balance's recovery stream to compute the correct
    // nullifier for the lookup.
    let mut initial_input_balance = input_balance.clone();
    initial_input_balance.recovery_stream.index -= 1;
    let input_balance_spent_nullifier = initial_input_balance.compute_nullifier();
    args.send_nullifier_spend_message(input_balance_spent_nullifier, receipt.transaction_hash)
        .await?;

    // Give some time for the message to be processed
    tokio::time::sleep(Duration::from_secs(3)).await;

    let darkpool = args.darkpool_instance();

    // Assert that the indexed intent is committed to the onchain Merkle tree
    let indexed_intent = args.get_intent_by_nullifier(state_intent0.compute_nullifier()).await?;
    assert_state_object_committed(&indexed_intent.intent, &darkpool).await?;

    // Assert that the indexed input balance is committed to the onchain Merkle tree
    let indexed_input_balance =
        args.get_balance_by_nullifier(input_balance.compute_nullifier()).await?;

    assert_state_object_committed(&indexed_input_balance.balance, &darkpool).await?;

    // Assert that the indexed output balance is committed to the onchain Merkle
    // tree
    let indexed_output_balance =
        args.get_balance_by_nullifier(out_balance0.compute_nullifier()).await?;

    assert_state_object_committed(&indexed_output_balance.balance, &darkpool).await?;

    Ok(())
}
indexer_integration_test!(test_ring2_first_fill);

// -----------
// | Helpers |
// -----------

/// Submit the settlement of the first fill of a ring 2 intent.
///
/// This will settle the fill into a new output balance for party 0. There is no
/// meaningful difference in indexing logic between this, and settling a first
/// ring2 fill into an existing output balance, so we always opt for this case
/// for simplicity.
///
/// Returns the transaction receipt, along with party 0's newly-created intent &
/// output balance.
async fn submit_ring2_first_fill(
    args: &mut TestArgs,
    in_balance: &mut DarkpoolStateBalance,
) -> Result<(TransactionReceipt, DarkpoolStateIntent, DarkpoolStateBalance)> {
    // Build the crossing intents & obligations
    let (intent0, intent1, obligation0, obligation1) = create_intents_and_obligations(args)?;

    // Split the obligations in 2 to allow for 2 fills
    let (first_obligation0, _) = split_obligation(&obligation0);
    let (first_obligation1, _) = split_obligation(&obligation1);

    let (state_intent0, out_balance0, settlement_bundle0) =
        build_ring2_settlement_bundle_first_fill(args, &intent0, &first_obligation0, in_balance)
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

    Ok((receipt, state_intent0, out_balance0))
}

/// Build party 0's settlement bundle for the first fill of a ring 2 intent.
///
/// This will prove validity for a new output balance for party 0, which the
/// bundle targets.
async fn build_ring2_settlement_bundle_first_fill(
    args: &mut TestArgs,
    intent: &Intent,
    obligation: &SettlementObligation,
    in_balance: &mut DarkpoolStateBalance,
) -> Result<(DarkpoolStateIntent, DarkpoolStateBalance, SettlementBundle)> {
    // Generate the validity proofs
    let commitment = in_balance.compute_commitment();
    let opening = fetch_merkle_opening(commitment, &args.darkpool_instance()).await?;

    let (state_intent, validity_statement, validity_proof, validity_link_hint) =
        generate_intent_and_balance_first_fill_validity_proof(args, intent, in_balance, &opening)?;

    let (
        out_balance,
        new_output_balance_statement,
        new_output_balance_proof,
        new_output_balance_link_hint,
    ) = generate_new_output_balance_validity_proof(args, obligation, in_balance, &opening)?;

    // Generate the settlement proof
    let (settlement_statement, settlement_proof, settlement_link_hint) =
        generate_ring2_settlement_proof(&state_intent, in_balance, &out_balance, obligation)?;

    // Build the auth bundles
    let auth_bundle =
        build_renegade_settled_first_fill_auth_bundle(&validity_statement, &validity_proof)?;

    let validity_link_proof =
        link_sized_intent_and_balance_settlement(&validity_link_hint, &settlement_link_hint)?;

    let output_balance_link_proof =
        link_sized_output_balance_settlement(&new_output_balance_link_hint, &settlement_link_hint)?;

    let new_output_auth_bundle = OutputBalanceBundle::new_output_balance(
        U256::from(MERKLE_HEIGHT),
        new_output_balance_statement.into(),
        new_output_balance_proof.into(),
        output_balance_link_proof.into(),
    );

    let settlement_bundle = SettlementBundle::renegade_settled_private_intent_first_fill(
        auth_bundle,
        new_output_auth_bundle,
        settlement_statement.into(),
        settlement_proof.into(),
        validity_link_proof.into(),
    );

    Ok((state_intent, out_balance, settlement_bundle))
}

/// Generate a validity proof for the first fill of a private intent & balance
/// for party 0
fn generate_intent_and_balance_first_fill_validity_proof(
    args: &mut TestArgs,
    intent: &Intent,
    balance: &mut DarkpoolStateBalance,
    balance_opening: &MerkleAuthenticationPath,
) -> Result<(
    DarkpoolStateIntent,
    IntentAndBalanceFirstFillValidityStatement,
    PlonkProof,
    ProofLinkingHint,
)> {
    let (witness, statement, state_intent) =
        create_intent_and_balance_first_fill_validity_witness_statement(
            args,
            intent,
            balance,
            balance_opening,
        );

    let (proof, link_hint) = singleprover_prove_with_hint::<
        SizedIntentAndBalanceFirstFillValidityCircuit,
    >(&witness, &statement)?;

    Ok((state_intent, statement, proof, link_hint))
}

/// Create a witness and statement for the
/// `IntentAndBalanceFirstFillValidityCircuit` using the given intent and
/// balance, assuming they are owned by party 0.
///
/// Returns the witness, statement, and the new (post-mutation) state intent.
fn create_intent_and_balance_first_fill_validity_witness_statement(
    args: &mut TestArgs,
    intent: &Intent,
    balance: &mut DarkpoolStateBalance,
    balance_opening: &MerkleAuthenticationPath,
) -> (
    SizedIntentAndBalanceFirstFillValidityWitness,
    IntentAndBalanceFirstFillValidityStatement,
    DarkpoolStateIntent,
) {
    let share_stream_seed = args.next_party0_share_stream().seed;
    let recovery_stream_seed = args.next_party0_recovery_stream().seed;

    let initial_intent =
        DarkpoolStateIntent::new(intent.clone(), share_stream_seed, recovery_stream_seed);

    let old_balance = balance.clone();

    let old_balance_nullifier = old_balance.compute_nullifier();

    // On the first fill, we don't re-encrypt the intent's amount share
    let new_amount_public_share = initial_intent.public_share.amount_in;

    // Re-encrypt the post-match balance shares
    let post_match_balance_shares = balance.reencrypt_post_match_share();

    // TODO: Authority handling changed from Address to SchnorrPublicKey.
    // The new system uses a Schnorr signature to authorize the intent from the
    // balance. Creating a dummy signature for now - this test will need proper
    // integration with the new authorization flow using actual Schnorr signing.
    let dummy_key = SchnorrPrivateKey::random();
    // Sign something arbitrary to create a valid-shaped signature
    let intent_authorization_signature =
        dummy_key.sign(&[renegade_constants::Scalar::default()]).expect("signing should not fail");

    // Construct the witness
    let witness = IntentAndBalanceFirstFillValidityWitness {
        intent: intent.clone(),
        initial_intent_share_stream: initial_intent.share_stream.clone(),
        initial_intent_recovery_stream: initial_intent.recovery_stream.clone(),
        private_intent_shares: initial_intent.private_shares(),
        new_amount_public_share,
        intent_authorization_signature,
        balance: old_balance.inner.clone(),
        old_balance,
        post_match_balance_shares,
        balance_opening: balance_opening.clone().into(),
    };

    let mut state_intent = initial_intent.clone();

    let intent_public_share = PreMatchIntentShare::from(state_intent.public_share());
    let intent_recovery_id = state_intent.compute_recovery_id();
    let intent_private_share_commitment = state_intent.compute_private_commitment();

    let balance_recovery_id = balance.compute_recovery_id();
    let balance_partial_commitment =
        balance.compute_partial_commitment(BALANCE_PARTIAL_COMMITMENT_SIZE);

    // Construct the statement
    let statement = IntentAndBalanceFirstFillValidityStatement {
        merkle_root: balance_opening.compute_root(),
        intent_public_share,
        intent_private_share_commitment,
        intent_recovery_id,
        balance_partial_commitment,
        old_balance_nullifier,
        balance_recovery_id,
    };

    (witness, statement, state_intent)
}

/// Generate a validity proof for a new output balance, assuming it is owned by
/// party 0.
///
/// TODO: The NewOutputBalanceValidityCircuit has been significantly
/// restructured. It now requires:
/// - An existing balance to bootstrap authorization from
/// - A Merkle opening for the existing balance
/// - A Schnorr signature from the existing balance's authority
///
/// This function needs to be rewritten to match the new circuit design.
/// For now, it's stubbed to allow compilation but will fail at runtime.
fn generate_new_output_balance_validity_proof(
    args: &mut TestArgs,
    obligation: &SettlementObligation,
    existing_balance: &DarkpoolStateBalance,
    existing_balance_opening: &MerkleAuthenticationPath,
) -> Result<(DarkpoolStateBalance, NewOutputBalanceValidityStatement, PlonkProof, ProofLinkingHint)>
{
    let owner = args.party0_address();
    let new_balance = DarkpoolBalance::new(
        obligation.output_token,
        owner,
        Address::random(), // relayer_fee_recipient
        SchnorrPrivateKey::random().public_key(),
    );

    let share_stream_seed = args.next_party0_share_stream().seed;
    let recovery_stream_seed = args.next_party0_recovery_stream().seed;

    let mut state_balance =
        DarkpoolStateBalance::new(new_balance.clone(), share_stream_seed, recovery_stream_seed);

    // Compute values needed for statement
    let recovery_id = state_balance.compute_recovery_id();
    let new_balance_partial_commitment =
        state_balance.compute_partial_commitment(NEW_BALANCE_PARTIAL_COMMITMENT_SIZE);
    let pre_match_balance_shares = PreMatchBalanceShare::from(state_balance.public_share.clone());
    let post_match_balance_shares = PostMatchBalanceShare::from(state_balance.public_share.clone());

    // Create dummy authorization signature
    // TODO: This needs to be a real signature from the existing balance's authority
    // key
    let dummy_key = SchnorrPrivateKey::random();
    let new_balance_authorization_signature =
        dummy_key.sign(&[renegade_constants::Scalar::default()]).expect("signing should not fail");

    // Build the witness
    let witness = SizedNewOutputBalanceValidityWitness {
        new_balance: state_balance.clone(),
        balance: new_balance,
        post_match_balance_shares,
        existing_balance: existing_balance.clone(),
        existing_balance_opening: existing_balance_opening.clone().into(),
        new_balance_authorization_signature,
    };

    // Build the statement
    let existing_balance_nullifier = existing_balance.compute_nullifier();
    let statement = NewOutputBalanceValidityStatement {
        existing_balance_merkle_root: existing_balance_opening.compute_root(),
        existing_balance_nullifier,
        pre_match_balance_shares,
        new_balance_partial_commitment,
        recovery_id,
    };

    let (proof, link_hint) =
        singleprover_prove_with_hint::<SizedNewOutputBalanceValidityCircuit>(&witness, &statement)?;

    // Advance the recovery stream to match in-circuit updates
    state_balance.recovery_stream.advance_by(1);

    Ok((state_balance, statement, proof, link_hint))
}

/// Generate a settlement proof for a ring 2 fill
fn generate_ring2_settlement_proof(
    intent: &DarkpoolStateIntent,
    input_balance: &DarkpoolStateBalance,
    output_balance: &DarkpoolStateBalance,
    obligation: &SettlementObligation,
) -> Result<(IntentAndBalancePublicSettlementStatement, PlonkProof, ProofLinkingHint)> {
    let pre_settlement_amount_public_share = intent.public_share.amount_in;
    let in_balance_public_shares = PostMatchBalanceShare::from(input_balance.public_share());
    let out_balance_public_shares = PostMatchBalanceShare::from(output_balance.public_share());

    let witness = IntentAndBalancePublicSettlementWitness {
        intent: intent.inner.clone(),
        pre_settlement_amount_public_share,
        in_balance: input_balance.inner.clone(),
        pre_settlement_in_balance_shares: in_balance_public_shares.clone(),
        out_balance: output_balance.inner.clone(),
        pre_settlement_out_balance_shares: out_balance_public_shares.clone(),
    };

    let statement = IntentAndBalancePublicSettlementStatement {
        settlement_obligation: obligation.clone(),
        amount_public_share: pre_settlement_amount_public_share,
        in_balance_public_shares,
        out_balance_public_shares,
        relayer_fee: settlement_relayer_fee(),
        relayer_fee_recipient: output_balance.inner.relayer_fee_recipient,
    };

    // Prove the relation
    let (proof, link_hint) = singleprover_prove_with_hint::<IntentAndBalancePublicSettlementCircuit>(
        &witness, &statement,
    )?;

    Ok((statement, proof, link_hint))
}

/// Build an auth bundle for the first fill of a private intent & balance
fn build_renegade_settled_first_fill_auth_bundle(
    validity_statement: &IntentAndBalanceFirstFillValidityStatement,
    validity_proof: &PlonkProof,
) -> Result<RenegadeSettledIntentAuthBundleFirstFill> {
    Ok(RenegadeSettledIntentAuthBundleFirstFill {
        merkleDepth: U256::from(MERKLE_HEIGHT),
        statement: validity_statement.clone().into(),
        validityProof: validity_proof.clone().into(),
    })
}
