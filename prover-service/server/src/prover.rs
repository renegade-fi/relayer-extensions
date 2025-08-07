//! The prover service implementation

// ---------------------
// | Endpoint Handlers |
// ---------------------

use prover_service_api::{
    LinkCommitmentsReblindRequest, ProofAndHintResponse, ProofAndLinkResponse, ProofLinkResponse,
    ProofResponse, ValidCommitmentsRequest, ValidFeeRedemptionRequest,
    ValidMalleableMatchSettleAtomicRequest, ValidMatchSettleAtomicRequest, ValidMatchSettleRequest,
    ValidMatchSettleResponse, ValidOfflineFeeSettlementRequest, ValidReblindRequest,
    ValidWalletCreateRequest, ValidWalletUpdateRequest,
};
use renegade_circuit_types::{
    PlonkLinkProof, PlonkProof, ProofLinkingHint, errors::ProverError, traits::SingleProverCircuit,
};
use renegade_circuits::{
    singleprover_prove_with_hint,
    zk_circuits::{
        proof_linking::{
            link_sized_commitments_atomic_match_settle, link_sized_commitments_match_settle,
            link_sized_commitments_reblind,
        },
        valid_commitments::SizedValidCommitments,
        valid_fee_redemption::SizedValidFeeRedemption,
        valid_malleable_match_settle_atomic::SizedValidMalleableMatchSettleAtomic,
        valid_match_settle::SizedValidMatchSettle,
        valid_match_settle_atomic::SizedValidMatchSettleAtomic,
        valid_offline_fee_settlement::SizedValidOfflineFeeSettlement,
        valid_reblind::SizedValidReblind,
        valid_wallet_create::SizedValidWalletCreate,
        valid_wallet_update::SizedValidWalletUpdate,
    },
};
use tracing::instrument;
use warp::{reject::Rejection, reply::Json};

use crate::error::ProverServiceError;

/// Handle a request to prove `VALID WALLET CREATE`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_wallet_create(
    request: ValidWalletCreateRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidWalletCreate>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID WALLET UPDATE`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_wallet_update(
    request: ValidWalletUpdateRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidWalletUpdate>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID COMMITMENTS`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_commitments(
    request: ValidCommitmentsRequest,
) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<SizedValidCommitments>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID REBLIND`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_reblind(request: ValidReblindRequest) -> Result<Json, Rejection> {
    generate_proof_and_hint_json::<SizedValidReblind>(request.witness, request.statement).await
}

/// Handle a request to generate a proof-link of `VALID COMMITMENTS` <-> `VALID
/// REBLIND`
#[instrument(skip_all)]
pub(crate) async fn handle_link_commitments_reblind(
    request: LinkCommitmentsReblindRequest,
) -> Result<Json, Rejection> {
    let link_proof =
        link_reblind_commitments(request.valid_reblind_hint, request.valid_commitments_hint)
            .await?;

    let resp = ProofLinkResponse { link_proof };
    Ok(warp::reply::json(&resp))
}

/// Handle a request to prove `VALID MATCH SETTLE`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_match_settle(
    request: ValidMatchSettleRequest,
) -> Result<Json, Rejection> {
    // Prove `VALID MATCH SETTLE` and generate a link hint
    let (plonk_proof, link_hint) =
        prove_circuit::<SizedValidMatchSettle>(request.witness, request.statement).await?;

    // Generate the link proofs in parallel
    let hint = link_hint.clone();
    let proof0_fut = run_blocking(move || {
        link_sized_commitments_match_settle(
            0, // party
            &request.valid_commitments_hint0,
            &hint,
        )
    });

    let proof1_fut = run_blocking(move || {
        link_sized_commitments_match_settle(
            1, // party
            &request.valid_commitments_hint1,
            &link_hint.clone(),
        )
    });

    // Join the proofs
    let (maybe_proof0, maybe_proof1) = tokio::join!(proof0_fut, proof1_fut);
    let (link_proof0, link_proof1) = (maybe_proof0?, maybe_proof1?);
    let resp = ValidMatchSettleResponse { plonk_proof, link_proof0, link_proof1 };
    Ok(warp::reply::json(&resp))
}

/// Handle a request to prove `VALID MATCH SETTLE ATOMIC`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_match_settle_atomic(
    request: ValidMatchSettleAtomicRequest,
) -> Result<Json, Rejection> {
    // Prove `VALID MATCH SETTLE ATOMIC` and generate a link hint
    let (plonk_proof, link_hint) =
        prove_circuit::<SizedValidMatchSettleAtomic>(request.witness, request.statement).await?;

    // Generate the link proof
    let link_proof =
        link_commitments_match_settle_atomic(request.valid_commitments_hint, link_hint).await?;
    let resp = ProofAndLinkResponse { plonk_proof, link_proof };
    Ok(warp::reply::json(&resp))
}

/// Handle a request to prove `VALID MALLEABLE MATCH SETTLE ATOMIC`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_malleable_match_settle_atomic(
    request: ValidMalleableMatchSettleAtomicRequest,
) -> Result<Json, Rejection> {
    // Prove `VALID MALLEABLE MATCH SETTLE ATOMIC` and generate a link hint
    let (plonk_proof, link_hint) =
        prove_circuit::<SizedValidMalleableMatchSettleAtomic>(request.witness, request.statement)
            .await?;

    // Generate the link proof
    let link_proof =
        link_commitments_malleable_match_settle_atomic(request.valid_commitments_hint, link_hint)
            .await?;
    let resp = ProofAndLinkResponse { plonk_proof, link_proof };
    Ok(warp::reply::json(&resp))
}

/// Handle a request to prove `VALID FEE REDEMPTION`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_fee_redemption(
    request: ValidFeeRedemptionRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidFeeRedemption>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID OFFLINE FEE SETTLEMENT`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_offline_fee_settlement(
    request: ValidOfflineFeeSettlementRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidOfflineFeeSettlement>(request.witness, request.statement).await
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
    C::Witness: 'static + Send,
    C::Statement: 'static + Send,
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
    C::Witness: 'static + Send,
    C::Statement: 'static + Send,
{
    let (proof, link_hint) = prove_circuit::<C>(witness, statement).await?;
    let resp = ProofAndHintResponse { proof, link_hint };
    Ok(warp::reply::json(&resp))
}

/// Prove a circuit; return the proof and link hint
#[instrument(skip_all, fields(circuit = C::name()))]
async fn prove_circuit<C: SingleProverCircuit>(
    witness: C::Witness,
    statement: C::Statement,
) -> Result<(PlonkProof, ProofLinkingHint), Rejection>
where
    C::Witness: 'static + Send,
    C::Statement: 'static + Send,
{
    run_blocking(move || singleprover_prove_with_hint::<C>(witness, statement)).await
}

// --- Proof Linking --- //

/// Generate a proof-link of `VALID COMMITMENTS` <-> `VALID REBLIND`
#[instrument(skip_all)]
async fn link_reblind_commitments(
    reblind_hint: ProofLinkingHint,
    commitments_hint: ProofLinkingHint,
) -> Result<PlonkLinkProof, Rejection> {
    run_blocking(move || link_sized_commitments_reblind(&reblind_hint, &commitments_hint)).await
}

/// Generate a proof-link of `VALID COMMITMENTS` <-> `VALID MATCH SETTLE ATOMIC`
#[instrument(skip_all)]
async fn link_commitments_match_settle_atomic(
    commitments_hint: ProofLinkingHint,
    match_settle_hint: ProofLinkingHint,
) -> Result<PlonkLinkProof, Rejection> {
    run_blocking(move || {
        link_sized_commitments_atomic_match_settle(&commitments_hint, &match_settle_hint)
    })
    .await
}

/// Generate a proof-link of `VALID COMMITMENTS` <-> `VALID MALLEABLE MATCH
/// SETTLE ATOMIC`
#[instrument(skip_all)]
async fn link_commitments_malleable_match_settle_atomic(
    commitments_hint: ProofLinkingHint,
    malleable_match_settle_hint: ProofLinkingHint,
) -> Result<PlonkLinkProof, Rejection> {
    run_blocking(move || {
        link_sized_commitments_atomic_match_settle(&commitments_hint, &malleable_match_settle_hint)
    })
    .await
}

// --- Runtime --- //

/// Block on a prover callback and handle errors
async fn run_blocking<F, R>(f: F) -> Result<R, Rejection>
where
    F: FnOnce() -> Result<R, ProverError> + Send + 'static,
    R: Send + 'static,
{
    let r = tokio::task::spawn_blocking(f)
        .await
        .map_err(ProverServiceError::custom)? // join error
        .map_err(ProverServiceError::prover)?; // proof system error

    Ok(r)
}
