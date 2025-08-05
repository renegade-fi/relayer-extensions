//! The prover service implementation

// ---------------------
// | Endpoint Handlers |
// ---------------------

use prover_service_api::{
    ValidCommitmentsRequest, ValidFeeRedemptionRequest, ValidMalleableMatchSettleAtomicRequest,
    ValidMatchSettleAtomicRequest, ValidMatchSettleRequest, ValidOfflineFeeSettlementRequest,
    ValidReblindRequest, ValidWalletCreateRequest, ValidWalletUpdateRequest,
};
use warp::{reject::Rejection, reply::Json};

/// Handle a request to prove `VALID WALLET CREATE`
pub(crate) async fn handle_valid_wallet_create(
    request: ValidWalletCreateRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-wallet-create"
    })))
}

/// Handle a request to prove `VALID WALLET UPDATE`
pub(crate) async fn handle_valid_wallet_update(
    request: ValidWalletUpdateRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-wallet-update"
    })))
}

/// Handle a request to prove `VALID COMMITMENTS`
pub(crate) async fn handle_valid_commitments(
    request: ValidCommitmentsRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-commitments"
    })))
}

/// Handle a request to prove `VALID REBLIND`
pub(crate) async fn handle_valid_reblind(request: ValidReblindRequest) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-reblind"
    })))
}

/// Handle a request to prove `VALID MATCH SETTLE`
pub(crate) async fn handle_valid_match_settle(
    request: ValidMatchSettleRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-match-settle"
    })))
}

/// Handle a request to prove `VALID MATCH SETTLE ATOMIC`
pub(crate) async fn handle_valid_match_settle_atomic(
    request: ValidMatchSettleAtomicRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-match-settle-atomic"
    })))
}

/// Handle a request to prove `VALID MALLEABLE MATCH SETTLE ATOMIC`
pub(crate) async fn handle_valid_malleable_match_settle_atomic(
    request: ValidMalleableMatchSettleAtomicRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-malleable-match-settle-atomic"
    })))
}

/// Handle a request to prove `VALID FEE REDEMPTION`
pub(crate) async fn handle_valid_fee_redemption(
    request: ValidFeeRedemptionRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-fee-redemption"
    })))
}

/// Handle a request to prove `VALID OFFLINE FEE SETTLEMENT`
pub(crate) async fn handle_valid_offline_fee_settlement(
    request: ValidOfflineFeeSettlementRequest,
) -> Result<Json, Rejection> {
    Ok(warp::reply::json(&serde_json::json!({
        "msg": "valid-offline-fee-settlement"
    })))
}
