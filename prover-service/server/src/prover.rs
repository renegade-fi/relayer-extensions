//! The prover service implementation

// ---------------------
// | Endpoint Handlers |
// ---------------------

use prover_service_api::{
    IntentAndBalanceBoundedSettlementRequest, IntentAndBalanceFirstFillValidityRequest,
    IntentAndBalancePrivateSettlementRequest, IntentAndBalancePublicSettlementRequest,
    IntentAndBalanceValidityRequest, IntentOnlyBoundedSettlementRequest,
    IntentOnlyFirstFillValidityRequest, IntentOnlyPublicSettlementRequest,
    IntentOnlyValidityRequest, LinkIntentAndBalanceSettlementRequest,
    LinkIntentOnlySettlementRequest, LinkOutputBalanceSettlementRequest,
    NewOutputBalanceValidityRequest, OutputBalanceValidityRequest, ProofAndHintResponse,
    ProofLinkResponse, ProofResponse, ValidBalanceCreateRequest, ValidDepositRequest,
    ValidNoteRedemptionRequest, ValidOrderCancellationRequest,
    ValidPrivateProtocolFeePaymentRequest, ValidPrivateRelayerFeePaymentRequest,
    ValidPublicProtocolFeePaymentRequest, ValidPublicRelayerFeePaymentRequest,
    ValidWithdrawalRequest,
};
use renegade_circuit_types::{PlonkProof, ProofLinkingHint, traits::SingleProverCircuit};
use renegade_circuits_core::{
    singleprover_prove_with_hint,
    zk_circuits::{
        // Fee proofs
        fees::{
            valid_note_redemption::SizedValidNoteRedemption,
            valid_private_protocol_fee_payment::SizedValidPrivateProtocolFeePayment,
            valid_private_relayer_fee_payment::SizedValidPrivateRelayerFeePayment,
            valid_public_protocol_fee_payment::SizedValidPublicProtocolFeePayment,
            valid_public_relayer_fee_payment::SizedValidPublicRelayerFeePayment,
        },
        // Proof linking
        proof_linking::{
            intent_and_balance::link_sized_intent_and_balance_settlement_with_party,
            intent_only::link_sized_intent_only_settlement,
            output_balance::link_sized_output_balance_settlement_with_party,
        },
        // Settlement proofs
        settlement::{
            intent_and_balance_bounded_settlement::IntentAndBalanceBoundedSettlementCircuit,
            intent_and_balance_private_settlement::IntentAndBalancePrivateSettlementCircuit,
            intent_and_balance_public_settlement::IntentAndBalancePublicSettlementCircuit,
            intent_only_bounded_settlement::IntentOnlyBoundedSettlementCircuit,
            intent_only_public_settlement::IntentOnlyPublicSettlementCircuit,
        },
        // Update proofs
        valid_balance_create::ValidBalanceCreate,
        valid_deposit::SizedValidDeposit,
        valid_order_cancellation::SizedValidOrderCancellationCircuit,
        valid_withdrawal::SizedValidWithdrawal,
        // Validity proofs
        validity_proofs::{
            intent_and_balance::SizedIntentAndBalanceValidityCircuit,
            intent_and_balance_first_fill::SizedIntentAndBalanceFirstFillValidityCircuit,
            intent_only::SizedIntentOnlyValidityCircuit,
            intent_only_first_fill::IntentOnlyFirstFillValidityCircuit,
            new_output_balance::SizedNewOutputBalanceValidityCircuit,
            output_balance::SizedOutputBalanceValidityCircuit,
        },
    },
};
use serde::Serialize;
use tracing::instrument;
use warp::{reject::Rejection, reply::Json};

use crate::error::ProverServiceError;

// --- Update Proof Handlers --- //

/// Handle a request to prove `VALID BALANCE CREATE`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_balance_create(
    request: ValidBalanceCreateRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<ValidBalanceCreate>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID DEPOSIT`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_deposit(request: ValidDepositRequest) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidDeposit>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID ORDER CANCELLATION`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_order_cancellation(
    request: ValidOrderCancellationRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidOrderCancellationCircuit>(request.witness, request.statement)
        .await
}

/// Handle a request to prove `VALID WITHDRAWAL`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_withdrawal(
    request: ValidWithdrawalRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidWithdrawal>(request.witness, request.statement).await
}

// --- Validity Proof Handlers --- //

/// Handle a request to prove `INTENT AND BALANCE VALIDITY`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_and_balance_validity(
    request: IntentAndBalanceValidityRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<SizedIntentAndBalanceValidityCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `INTENT AND BALANCE FIRST FILL VALIDITY`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_and_balance_first_fill_validity(
    request: IntentAndBalanceFirstFillValidityRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<SizedIntentAndBalanceFirstFillValidityCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `INTENT ONLY VALIDITY`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_only_validity(
    request: IntentOnlyValidityRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<SizedIntentOnlyValidityCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `INTENT ONLY FIRST FILL VALIDITY`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_only_first_fill_validity(
    request: IntentOnlyFirstFillValidityRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<IntentOnlyFirstFillValidityCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `NEW OUTPUT BALANCE VALIDITY`
#[instrument(skip_all)]
pub(crate) async fn handle_new_output_balance_validity(
    request: NewOutputBalanceValidityRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<SizedNewOutputBalanceValidityCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `OUTPUT BALANCE VALIDITY`
#[instrument(skip_all)]
pub(crate) async fn handle_output_balance_validity(
    request: OutputBalanceValidityRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<SizedOutputBalanceValidityCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

// --- Settlement Proof Handlers --- //

/// Handle a request to prove `INTENT AND BALANCE BOUNDED SETTLEMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_and_balance_bounded_settlement(
    request: IntentAndBalanceBoundedSettlementRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<IntentAndBalanceBoundedSettlementCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `INTENT AND BALANCE PRIVATE SETTLEMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_and_balance_private_settlement(
    request: IntentAndBalancePrivateSettlementRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<IntentAndBalancePrivateSettlementCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `INTENT AND BALANCE PUBLIC SETTLEMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_and_balance_public_settlement(
    request: IntentAndBalancePublicSettlementRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<IntentAndBalancePublicSettlementCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `INTENT ONLY BOUNDED SETTLEMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_only_bounded_settlement(
    request: IntentOnlyBoundedSettlementRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<IntentOnlyBoundedSettlementCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

/// Handle a request to prove `INTENT ONLY PUBLIC SETTLEMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_intent_only_public_settlement(
    request: IntentOnlyPublicSettlementRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<IntentOnlyPublicSettlementCircuit>(
        request.witness,
        request.statement,
    )
    .await
}

// --- Fee Proof Handlers --- //

/// Handle a request to prove `VALID NOTE REDEMPTION`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_note_redemption(
    request: ValidNoteRedemptionRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidNoteRedemption>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID PRIVATE PROTOCOL FEE PAYMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_private_protocol_fee_payment(
    request: ValidPrivateProtocolFeePaymentRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidPrivateProtocolFeePayment>(request.witness, request.statement)
        .await
}

/// Handle a request to prove `VALID PRIVATE RELAYER FEE PAYMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_private_relayer_fee_payment(
    request: ValidPrivateRelayerFeePaymentRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidPrivateRelayerFeePayment>(request.witness, request.statement)
        .await
}

/// Handle a request to prove `VALID PUBLIC PROTOCOL FEE PAYMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_public_protocol_fee_payment(
    request: ValidPublicProtocolFeePaymentRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidPublicProtocolFeePayment>(request.witness, request.statement)
        .await
}

/// Handle a request to prove `VALID PUBLIC RELAYER FEE PAYMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_public_relayer_fee_payment(
    request: ValidPublicRelayerFeePaymentRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidPublicRelayerFeePayment>(request.witness, request.statement)
        .await
}

// --- Proof Linking Handlers --- //

/// Handle a request to link intent and balance validity <-> settlement
#[instrument(skip_all)]
pub(crate) async fn handle_link_intent_and_balance_settlement(
    request: LinkIntentAndBalanceSettlementRequest,
) -> Result<Json, Rejection> {
    let link_proof = run_blocking(move || {
        link_sized_intent_and_balance_settlement_with_party(
            request.party_id,
            &request.validity_link_hint,
            &request.settlement_link_hint,
        )
    })
    .await?;

    let resp = ProofLinkResponse { link_proof };
    Ok(warp::reply::json(&resp))
}

/// Handle a request to link intent only validity <-> settlement
#[instrument(skip_all)]
pub(crate) async fn handle_link_intent_only_settlement(
    request: LinkIntentOnlySettlementRequest,
) -> Result<Json, Rejection> {
    let link_proof = run_blocking(move || {
        link_sized_intent_only_settlement(
            &request.validity_link_hint,
            &request.settlement_link_hint,
        )
    })
    .await?;

    let resp = ProofLinkResponse { link_proof };
    Ok(warp::reply::json(&resp))
}

/// Handle a request to link output balance validity <-> settlement
#[instrument(skip_all)]
pub(crate) async fn handle_link_output_balance_settlement(
    request: LinkOutputBalanceSettlementRequest,
) -> Result<Json, Rejection> {
    let link_proof = run_blocking(move || {
        link_sized_output_balance_settlement_with_party(
            request.party_id,
            &request.validity_link_hint,
            &request.settlement_link_hint,
        )
    })
    .await?;

    let resp = ProofLinkResponse { link_proof };
    Ok(warp::reply::json(&resp))
}

// -----------
// | Helpers |
// -----------

// --- Plonk Prover --- //

/// Prove a circuit and return a json-ified proof
async fn generate_proof_json<C: SingleProverCircuit>(
    witness: C::Witness,
    statement: C::Statement,
) -> Result<Json, Rejection>
where
    C::Witness: 'static + Send + Serialize,
    C::Statement: 'static + Send + Serialize,
{
    // Spawn on a blocking thread
    let (proof, _link_hint) = prove_circuit::<C>(witness, statement).await?;
    let resp = ProofResponse { proof };
    Ok(warp::reply::json(&resp))
}

/// Prove a circuit and return a json-ified proof and link hint
async fn generate_proof_and_hint_json<C: SingleProverCircuit>(
    witness: C::Witness,
    statement: C::Statement,
) -> Result<Json, Rejection>
where
    C::Witness: 'static + Send + Serialize,
    C::Statement: 'static + Send + Serialize,
{
    let (proof, link_hint) = prove_circuit::<C>(witness, statement).await?;
    let resp = ProofAndHintResponse { proof, link_hint };
    Ok(warp::reply::json(&resp))
}

// --- Runtime --- //

/// Prove a circuit in a blocking thread and log invalid bundles
#[cfg(feature = "log-invalid-bundles")]
async fn prove_circuit<C: SingleProverCircuit>(
    witness: C::Witness,
    statement: C::Statement,
) -> Result<(PlonkProof, ProofLinkingHint), Rejection>
where
    C::Witness: 'static + Send + Serialize,
    C::Statement: 'static + Send + Serialize,
{
    use mpc_plonk::errors::{PlonkError, SnarkError};
    use renegade_circuit_types::errors::ProverError;
    use tracing::error;

    run_blocking(move || {
        // Prove the circuit
        let res = singleprover_prove_with_hint::<C>(&witness, &statement);

        // Check for constraint satisfaction errors
        if let Err(ProverError::Plonk(PlonkError::SnarkError(
            SnarkError::WrongQuotientPolyDegree(..),
        ))) = &res
        {
            // Log the invalid witness and statement
            // Unwraps here are valid as the witness and statement were serialized across
            // the API
            let witness_json = serde_json::to_string(&witness).unwrap();
            let statement_json = serde_json::to_string(&statement).unwrap();
            error!(
                witness = %witness_json,
                statement = %statement_json,
                "Invalid witness/statement for circuit {}", C::name(),
            );
        }

        res
    })
    .await
}

/// Prove a circuit in a blocking thread, don't log invalid bundles
#[cfg(not(feature = "log-invalid-bundles"))]
async fn prove_circuit<C: SingleProverCircuit>(
    witness: C::Witness,
    statement: C::Statement,
) -> Result<(PlonkProof, ProofLinkingHint), Rejection>
where
    C::Witness: 'static + Send + Serialize,
    C::Statement: 'static + Send + Serialize,
{
    run_blocking(move || singleprover_prove_with_hint::<C>(&witness, &statement)).await
}

/// Block on a prover callback and handle errors
async fn run_blocking<F, R, E>(f: F) -> Result<R, Rejection>
where
    F: FnOnce() -> Result<R, E> + Send + 'static,
    R: Send + 'static,
    E: ToString + Send + 'static,
{
    let r = tokio::task::spawn_blocking(f)
        .await
        .map_err(ProverServiceError::custom)? // join error
        .map_err(|e| {
            // Convert WrongQuotientPolyDegree errors to BadRequest instead of Prover error
            let err_str = e.to_string();
            if err_str.contains("WrongQuotientPolyDegree") {
                ProverServiceError::bad_request(format!("Invalid witness: {err_str}"))
            } else {
                ProverServiceError::prover(err_str)
            }
        })?;

    Ok(r)
}
