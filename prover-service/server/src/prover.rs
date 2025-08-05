//! The prover service implementation

// ---------------------
// | Endpoint Handlers |
// ---------------------

use prover_service_api::{
    LinkCommitmentsReblindRequest, ProofLinkResponse, ProofResponse, ValidCommitmentsRequest,
    ValidFeeRedemptionRequest, ValidMalleableMatchSettleAtomicRequest,
    ValidMatchSettleAtomicRequest, ValidMatchSettleRequest, ValidOfflineFeeSettlementRequest,
    ValidReblindRequest, ValidWalletCreateRequest, ValidWalletUpdateRequest,
};
use renegade_circuit_types::traits::SingleProverCircuit;
use renegade_circuits::{
    singleprover_prove_with_hint,
    zk_circuits::{
        proof_linking::link_sized_commitments_reblind, valid_commitments::SizedValidCommitments,
        valid_fee_redemption::SizedValidFeeRedemption,
        valid_malleable_match_settle_atomic::SizedValidMalleableMatchSettleAtomic,
        valid_match_settle::SizedValidMatchSettle,
        valid_match_settle_atomic::SizedValidMatchSettleAtomic,
        valid_offline_fee_settlement::SizedValidOfflineFeeSettlement,
        valid_reblind::SizedValidReblind, valid_wallet_create::SizedValidWalletCreate,
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
    generate_proof_json::<SizedValidCommitments>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID REBLIND`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_reblind(request: ValidReblindRequest) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidReblind>(request.witness, request.statement).await
}

/// Handle a request to generate a proof-link of `VALID COMMITMENTS` <-> `VALID
/// REBLIND`
#[instrument(skip_all)]
pub(crate) async fn handle_link_commitments_reblind(
    request: LinkCommitmentsReblindRequest,
) -> Result<Json, Rejection> {
    // Spawn on a blocking thread to avoid blocking the async pool
    let link_proof = tokio::task::spawn_blocking(move || {
        link_sized_commitments_reblind(&request.valid_reblind_hint, &request.valid_commitments_hint)
    })
    .await
    .map_err(ProverServiceError::custom)? // join error
    .map_err(ProverServiceError::prover)?; // proof system error

    let resp = ProofLinkResponse { link_proof };
    Ok(warp::reply::json(&resp))
}

/// Handle a request to prove `VALID MATCH SETTLE`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_match_settle(
    request: ValidMatchSettleRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidMatchSettle>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID MATCH SETTLE ATOMIC`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_match_settle_atomic(
    request: ValidMatchSettleAtomicRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidMatchSettleAtomic>(request.witness, request.statement).await
}

/// Handle a request to prove `VALID MALLEABLE MATCH SETTLE ATOMIC`
#[instrument(skip_all)]
pub(crate) async fn handle_valid_malleable_match_settle_atomic(
    request: ValidMalleableMatchSettleAtomicRequest,
) -> Result<Json, Rejection> {
    generate_proof_json::<SizedValidMalleableMatchSettleAtomic>(request.witness, request.statement)
        .await
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

/// Prove a circuit and return a json-ified proof
pub(crate) async fn generate_proof_json<C: SingleProverCircuit>(
    witness: C::Witness,
    statement: C::Statement,
) -> Result<Json, Rejection>
where
    C::Witness: 'static + Send,
    C::Statement: 'static + Send,
{
    // Spawn on a blocking thread
    let (proof, link_hint) = tokio::task::spawn_blocking(move || singleprover_prove_with_hint::<C>(witness, statement))
        .await
        .map_err(ProverServiceError::custom)? // join error
        .map_err(ProverServiceError::prover)?; // proof system error

    let resp = ProofResponse { proof, link_hint };
    Ok(warp::reply::json(&resp))
}
