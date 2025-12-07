//! Integration testing utilities for managing intents

use alloy::{
    primitives::{U256, keccak256},
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
};
use darkpool_indexer::api::http::handlers::get_all_active_user_state_objects;
use darkpool_indexer_api::types::http::ApiStateObject;
use eyre::Result;
use rand::{Rng, thread_rng};
use renegade_circuit_types::{
    Commitment, PlonkLinkProof, PlonkProof, ProofLinkingHint,
    fixed_point::FixedPoint,
    intent::{DarkpoolStateIntent, Intent},
    max_amount,
    settlement_obligation::SettlementObligation,
    state_wrapper::StateWrapper,
};
use renegade_circuits::{
    singleprover_prove_with_hint,
    test_helpers::{
        BOUNDED_MAX_AMT, compute_implied_price, compute_min_amount_out, random_price, random_scalar,
    },
    zk_circuits::{
        proof_linking::intent_only::link_sized_intent_only_settlement,
        settlement::intent_only_public_settlement::{
            self, IntentOnlyPublicSettlementStatement, SizedIntentOnlyPublicSettlementCircuit,
        },
        validity_proofs::{
            intent_only::{self, IntentOnlyValidityCircuit, IntentOnlyValidityStatement},
            intent_only_first_fill::{
                IntentOnlyFirstFillValidityCircuit, IntentOnlyFirstFillValidityStatement,
                IntentOnlyFirstFillValidityWitness,
            },
        },
    },
};
use renegade_common::types::merkle::MerkleAuthenticationPath;
use renegade_constants::MERKLE_HEIGHT;
use renegade_crypto::fields::scalar_to_u256;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ObligationBundle, PrivateIntentAuthBundle, PrivateIntentAuthBundleFirstFill,
        SettlementBundle,
    },
    auth_helpers::sign_with_nonce,
};

use crate::{
    test_args::TestArgs,
    utils::{merkle::parse_merkle_opening_from_receipt, transactions::wait_for_tx_success},
};

// -------------------------------
// | Ring 1 Intents / Settlement |
// -------------------------------

/// Submit a settlement between two ring 1 intents which both receive their
/// first fill.
///
/// Returns the transaction receipt, both intent state objects (after the first
/// fill has been applied to them), and both subsequent fill obligations
pub async fn submit_ring1_first_fill(
    args: &mut TestArgs,
) -> Result<(
    TransactionReceipt,
    DarkpoolStateIntent,
    DarkpoolStateIntent,
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

    let (mut state_intent1, settlement_bundle1) = build_ring1_settlement_bundle_first_fill(
        args,
        false, // is_party0
        &intent1,
        &first_obligation1,
    )?;

    let obligation_bundle = build_public_obligation_bundle(&first_obligation0, &first_obligation1);

    let darkpool = args.darkpool_instance();
    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);

    let receipt = wait_for_tx_success(call).await?;

    state_intent0.apply_settlement_obligation(&first_obligation0);
    state_intent1.apply_settlement_obligation(&first_obligation1);

    Ok((receipt, state_intent0, state_intent1, second_obligation0, second_obligation1))
}

/// Submit the settlement of a subsequent fill on the 2 given intents,
/// represented by the given 2 settlement obligations.
pub async fn submit_ring1_subsequent_fill(
    args: &TestArgs,
    state_intent0: &DarkpoolStateIntent,
    state_intent1: &DarkpoolStateIntent,
    obligation0: &SettlementObligation,
    obligation1: &SettlementObligation,
    receipt: &TransactionReceipt,
) -> Result<()> {
    let darkpool = args.darkpool_instance();

    let commitment0 = state_intent0.compute_commitment();
    let commitment1 = state_intent1.compute_commitment();

    let opening0 = parse_merkle_opening_from_receipt(commitment0, receipt)?;
    let opening1 = parse_merkle_opening_from_receipt(commitment1, receipt)?;

    let settlement_bundle0 =
        build_ring1_settlement_bundle_subsequent_fill(state_intent0, &opening0, obligation0)?;

    let settlement_bundle1 =
        build_ring1_settlement_bundle_subsequent_fill(state_intent1, &opening1, obligation1)?;

    let obligation_bundle = build_public_obligation_bundle(obligation0, obligation1);

    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);
    wait_for_tx_success(call).await?;

    Ok(())
}

/// Create two matching intents and obligations
///
/// Party 0 sells the base; party 1 sells the quote
pub fn create_intents_and_obligations(
    args: &TestArgs,
) -> Result<(Intent, Intent, SettlementObligation, SettlementObligation)> {
    // Construct a random intent for the first party
    let mut rng = thread_rng();
    let amount_in = rng.gen_range(0..=BOUNDED_MAX_AMT);
    let min_price = random_price();
    let intent0 = Intent {
        in_token: args.base_token_address()?,
        out_token: args.quote_token_address()?,
        owner: args.party0_address(),
        min_price,
        amount_in,
    };

    let counterparty = args.party1_address();

    // Determine the trade parameters
    let party0_amt_in = rng.gen_range(0..intent0.amount_in);
    let min_amt_out = compute_min_amount_out(&intent0, party0_amt_in);
    let party0_amt_out = rng.gen_range(min_amt_out..=max_amount());

    // Build two compatible obligations
    let obligation0 = SettlementObligation {
        input_token: intent0.in_token,
        output_token: intent0.out_token,
        amount_in: party0_amt_in,
        amount_out: party0_amt_out,
    };
    let obligation1 = SettlementObligation {
        input_token: intent0.out_token,
        output_token: intent0.in_token,
        amount_in: party0_amt_out,
        amount_out: party0_amt_in,
    };

    // Create a compatible intent for the counterparty
    let trade_price = compute_implied_price(obligation1.amount_out, obligation1.amount_in);

    let min_price = trade_price.floor_div(&FixedPoint::from(2_u128));
    let amount_in = rng.gen_range(party0_amt_out..=max_amount());
    let intent1 = Intent {
        in_token: intent0.out_token,
        out_token: intent0.in_token,
        owner: counterparty,
        min_price,
        amount_in,
    };

    Ok((intent0, intent1, obligation0, obligation1))
}

/// Build a settlement bundle for the first fill of a ring 1 intent
fn build_ring1_settlement_bundle_first_fill(
    args: &mut TestArgs,
    is_party0: bool,
    intent: &Intent,
    obligation: &SettlementObligation,
) -> Result<(DarkpoolStateIntent, SettlementBundle)> {
    // Generate proofs
    let (commitment, state_intent, validity_statement, validity_proof, validity_link_hint) =
        generate_ring1_first_fill_validity_proof(args, is_party0, intent)?;

    let (settlement_statement, settlement_proof, settlement_link_hint) =
        generate_ring1_settlement_proof(intent, obligation)?;

    let linking_proof = generate_ring1_linking_proof(&validity_link_hint, &settlement_link_hint)?;

    // Build bundles
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
) -> Result<(
    Commitment,
    DarkpoolStateIntent,
    IntentOnlyFirstFillValidityStatement,
    PlonkProof,
    ProofLinkingHint,
)> {
    // Build the witness and statement
    let (witness, statement) =
        create_intent_only_first_fill_validity_witness_statement(args, is_party0, intent);

    // Compute a commitment to the initial intent
    let intent = witness.intent.clone();
    let share_stream_seed = witness.initial_intent_share_stream.seed;
    let recovery_stream_seed = witness.initial_intent_recovery_stream.seed;
    let mut state_intent =
        DarkpoolStateIntent::new(intent, share_stream_seed, recovery_stream_seed);

    state_intent.compute_recovery_id();
    let comm = state_intent.compute_commitment();

    // Generate the validity proof
    let (proof, link_hint) = singleprover_prove_with_hint::<IntentOnlyFirstFillValidityCircuit>(
        witness,
        statement.clone(),
    )?;

    Ok((comm, state_intent, statement, proof, link_hint))
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
    let (proof, link_hint) = singleprover_prove_with_hint::<
        IntentOnlyValidityCircuit<MERKLE_HEIGHT>,
    >(witness, statement.clone())?;

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
        witness,
        statement.clone(),
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
    let comm_u256 = scalar_to_u256(&commitment);
    let comm_hash = keccak256(comm_u256.to_be_bytes_vec());
    let signature = sign_with_nonce(comm_hash.as_slice(), owner)?;

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

/// Build an obligation bundle for two public obligations
pub fn build_public_obligation_bundle(
    obligation0: &SettlementObligation,
    obligation1: &SettlementObligation,
) -> ObligationBundle {
    ObligationBundle::new_public(obligation0.clone().into(), obligation1.clone().into())
}

// ----------------
// | Misc Helpers |
// ----------------

/// Split an obligation in two
///
/// Returns the two splits of the obligation
pub fn split_obligation(
    obligation: &SettlementObligation,
) -> (SettlementObligation, SettlementObligation) {
    let mut obligation0 = obligation.clone();
    let mut obligation1 = obligation.clone();
    obligation0.amount_in /= 2;
    obligation0.amount_out /= 2;
    obligation1.amount_in /= 2;
    obligation1.amount_out /= 2;

    (obligation0, obligation1)
}

/// The settlement relayer fee to use for testing
pub fn settlement_relayer_fee() -> FixedPoint {
    FixedPoint::from_f64_round_down(0.0001) // 1bp
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

/// Create a witness and statement for the `IntentOnlyFirstFillValidityCircuit`
/// using the given intent
fn create_intent_only_first_fill_validity_witness_statement(
    args: &mut TestArgs,
    is_party0: bool,
    intent: &Intent,
) -> (IntentOnlyFirstFillValidityWitness, IntentOnlyFirstFillValidityStatement) {
    // Create the witness intent with initial stream states
    let (share_stream_seed, recovery_stream_seed) = if is_party0 {
        (args.next_party0_share_stream().seed, args.next_party0_recovery_stream().seed)
    } else {
        (random_scalar(), random_scalar())
    };

    let initial_intent = StateWrapper::new(intent.clone(), share_stream_seed, recovery_stream_seed);

    let mut intent_clone = initial_intent.clone();
    let recovery_id = intent_clone.compute_recovery_id();
    let intent_private_commitment = intent_clone.compute_private_commitment();

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

    (witness, statement)
}

/// Get the first intent state object for the first test account
pub async fn get_party0_first_intent(args: &TestArgs) -> Result<DarkpoolStateIntent> {
    let state_objects =
        get_all_active_user_state_objects(args.party0_account_id(), args.db_client()).await?;

    state_objects
        .into_iter()
        .find_map(|state_object| match state_object {
            ApiStateObject::Intent(intent) => Some(intent.intent),
            _ => None,
        })
        .ok_or(eyre::eyre!("Intent not found"))
}
