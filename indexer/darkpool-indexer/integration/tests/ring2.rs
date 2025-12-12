//! Tests the indexing of ring 2 (Renegade-settled, public-fill) match
//! settlements

use alloy::{
    primitives::{Address, U256},
    signers::local::PrivateKeySigner,
};
use eyre::Result;
use renegade_circuit_types::{
    Commitment, PlonkProof, ProofLinkingHint,
    balance::{Balance, DarkpoolStateBalance, PostMatchBalanceShare},
    intent::{DarkpoolStateIntent, Intent, PreMatchIntentShare},
    settlement_obligation::SettlementObligation,
};
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
                NewOutputBalanceValidityCircuit, NewOutputBalanceValidityStatement,
                test_helpers::create_witness_statement_with_balance,
            },
        },
    },
};
use renegade_common::types::merkle::MerkleAuthenticationPath;
use renegade_constants::MERKLE_HEIGHT;
use renegade_crypto::{fields::address_to_scalar, hash::compute_poseidon_hash};
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ObligationBundle, OutputBalanceBundle, RenegadeSettledIntentAuthBundleFirstFill,
        SettlementBundle,
    },
    auth_helpers::sign_with_nonce,
};

use crate::{
    test_args::TestArgs,
    tests::ring0::build_ring0_settlement_bundle,
    utils::{
        merkle::fetch_merkle_opening,
        test_data::{create_intents_and_obligations, settlement_relayer_fee, split_obligation},
        transactions::wait_for_tx_success,
    },
};

// -----------
// | Helpers |
// -----------

/// Submit the settlement of the first fill of a ring 2 intent.
///
/// This will settle the fill into a new output balance for party 0. There is no
/// meaningful difference in indexing logic between this, and settling a first
/// ring2 fill into an existing output balance, so we always opt for this case
/// for simplicity.
async fn submit_ring2_first_fill_new_output_balance(
    args: &mut TestArgs,
    in_balance: &mut DarkpoolStateBalance,
) -> Result<()> {
    // Build the crossing intents & obligations
    let (intent0, intent1, obligation0, obligation1) = create_intents_and_obligations(args)?;

    // Split the obligations in 2 to allow for 2 fills
    let (first_obligation0, _) = split_obligation(&obligation0);
    let (first_obligation1, _) = split_obligation(&obligation1);

    let (_, _, settlement_bundle0) =
        build_ring2_settlement_bundle_first_fill(args, &intent0, &first_obligation0, in_balance)
            .await?;

    let (settlement_bundle1, _) = build_ring0_settlement_bundle(
        args,
        false, // is_party0
        &intent1,
        &first_obligation1,
    )?;

    let obligation_bundle = ObligationBundle::new_public(
        first_obligation0.clone().into(),
        first_obligation1.clone().into(),
    );

    let darkpool = args.darkpool_instance();
    let call = darkpool.settleMatch(obligation_bundle, settlement_bundle0, settlement_bundle1);

    wait_for_tx_success(call).await?;

    Ok(())
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
    ) = generate_new_output_balance_validity_proof(args.party0_address(), obligation)?;

    // Generate the settlement proof
    let (settlement_statement, settlement_proof, settlement_link_hint) =
        generate_ring2_settlement_proof(&state_intent, in_balance, &out_balance, obligation)?;

    // Build the auth bundles
    let commitment = validity_statement.intent_and_authorizing_address_commitment;
    let auth_bundle = build_renegade_settled_first_fill_auth_bundle(
        &args.party0_signer(),
        commitment,
        &validity_statement,
        &validity_proof,
    )?;

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
    let initial_intent_share_stream = args.next_party0_share_stream();
    let share_stream_seed = initial_intent_share_stream.seed;

    let initial_intent_recovery_stream = args.next_party0_recovery_stream();
    let recovery_stream_seed = initial_intent_recovery_stream.seed;

    let initial_intent =
        DarkpoolStateIntent::new(intent.clone(), share_stream_seed, recovery_stream_seed);
    let old_balance = balance.clone();

    let initial_intent_commitment = initial_intent.compute_commitment();
    let old_balance_nullifier = old_balance.compute_nullifier();

    // On the first fill, we don't re-encrypt the intent's amount share
    let new_amount_public_share = initial_intent.public_share.amount_in;

    // Re-encrypt the post-match balance shares
    let new_one_time_address = balance.inner.one_time_authority;
    let new_one_time_share = balance.stream_cipher_encrypt(&new_one_time_address);
    let post_match_balance_shares = balance.reencrypt_post_match_share();
    balance.public_share.one_time_authority = new_one_time_share;

    // Construct the witness
    let witness = IntentAndBalanceFirstFillValidityWitness {
        intent: intent.clone(),
        initial_intent_share_stream,
        initial_intent_recovery_stream,
        private_intent_shares: initial_intent.private_shares(),
        new_amount_public_share,
        balance: old_balance.inner.clone(),
        old_balance,
        post_match_balance_shares,
        new_one_time_address,
        balance_opening: balance_opening.clone().into(),
    };

    let mut state_intent = initial_intent.clone();

    let intent_and_authorizing_address_commitment = compute_poseidon_hash(&[
        initial_intent_commitment,
        address_to_scalar(&new_one_time_address),
    ]);

    let intent_public_share = PreMatchIntentShare::from(state_intent.public_share());
    let intent_recovery_id = state_intent.compute_recovery_id();
    let intent_private_share_commitment = state_intent.compute_private_commitment();

    let balance_recovery_id = balance.compute_recovery_id();
    let balance_partial_commitment =
        balance.compute_partial_commitment(BALANCE_PARTIAL_COMMITMENT_SIZE);

    // Construct the statement
    let statement = IntentAndBalanceFirstFillValidityStatement {
        merkle_root: balance_opening.compute_root(),
        intent_and_authorizing_address_commitment,
        intent_public_share,
        intent_private_share_commitment,
        intent_recovery_id,
        balance_partial_commitment,
        new_one_time_address_public_share: new_one_time_share,
        old_balance_nullifier,
        balance_recovery_id,
        one_time_authorizing_address: new_one_time_address,
    };

    (witness, statement, state_intent)
}

/// Generate a validity proof for a new output balance
fn generate_new_output_balance_validity_proof(
    owner: Address,
    obligation: &SettlementObligation,
) -> Result<(DarkpoolStateBalance, NewOutputBalanceValidityStatement, PlonkProof, ProofLinkingHint)>
{
    let balance = Balance::new(
        obligation.output_token,
        owner,
        Address::random(), // relayer_fee_recipient
        owner,             // one_time_authority
    );

    let (witness, statement) = create_witness_statement_with_balance(balance.clone());
    let (proof, link_hint) =
        singleprover_prove_with_hint::<NewOutputBalanceValidityCircuit>(&witness, &statement)?;

    let share_seed = witness.initial_share_stream.seed;
    let recovery_seed = witness.initial_recovery_stream.seed;

    // Build the balance & advance the recovery stream to match the in-circuit
    // updates
    let mut state_balance = DarkpoolStateBalance::new(balance, share_seed, recovery_seed);
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
    owner: &PrivateKeySigner,
    commitment: Commitment,
    validity_statement: &IntentAndBalanceFirstFillValidityStatement,
    validity_proof: &PlonkProof,
) -> Result<RenegadeSettledIntentAuthBundleFirstFill> {
    let commitment_bytes = commitment.to_bytes_be();
    let signature = sign_with_nonce(&commitment_bytes, owner)?;

    Ok(RenegadeSettledIntentAuthBundleFirstFill {
        merkleDepth: U256::from(MERKLE_HEIGHT),
        ownerSignature: signature,
        statement: validity_statement.clone().into(),
        validityProof: validity_proof.clone().into(),
    })
}
