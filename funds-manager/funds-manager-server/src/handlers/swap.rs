//! Handlers for swap endpoints

use std::sync::Arc;

use funds_manager_api::quoters::{QuoteParams, SwapImmediateResponse, SwapIntoTargetTokenRequest};
use renegade_common::types::chain::Chain;
use tracing::{instrument, warn};
use warp::reply::Json;

use crate::{execution_client::error::ExecutionClientError, server::Server};

/// Handler for executing an immediate swap
#[instrument(skip_all)]
pub(crate) async fn swap_immediate_handler(
    chain: Chain,
    params: QuoteParams,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let execution_client = server.get_execution_client(&chain)?;
    let custody_client = server.get_custody_client(&chain)?;
    let metrics_recorder = server.get_metrics_recorder(&chain)?;

    // Top up the quoter hot wallet gas before swapping
    custody_client.top_up_quoter_hot_wallet_gas().await?;

    // Execute the swap, decaying the size of the swap each time it fails to execute
    let outcome = execution_client
        .swap_immediate_decaying(params)
        .await?
        .ok_or(ExecutionClientError::custom("No swap executed".to_string()))?;

    // Compute swap costs and respond
    let execution_cost = match metrics_recorder.record_swap_cost(&outcome).await {
        Ok(data) => data.execution_cost_usdc,
        Err(e) => {
            warn!("Failed to record swap cost metrics: {e}");
            0.0 // Default to 0 USD
        },
    };

    Ok(warp::reply::json(&SwapImmediateResponse {
        quote: outcome.quote.into(),
        tx_hash: format!("{:#x}", outcome.tx_hash),
        execution_cost,
    }))
}

/// Handler for executing a swap to cover a target amount of a given token
#[instrument(skip_all)]
pub(crate) async fn swap_into_target_token_handler(
    chain: Chain,
    req: SwapIntoTargetTokenRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let execution_client = server.get_execution_client(&chain)?;
    let custody_client = server.get_custody_client(&chain)?;
    let metrics_recorder = server.get_metrics_recorder(&chain)?;

    // Top up the quoter hot wallet gas before swapping
    custody_client.top_up_quoter_hot_wallet_gas().await?;

    // Execute the swap, decaying the size of the swap each time it fails to execute
    let outcomes = execution_client.try_swap_into_target_token(req).await?;

    // Compute swap costs and respond
    let mut responses = vec![];
    for outcome in outcomes {
        let execution_cost = match metrics_recorder.record_swap_cost(&outcome).await {
            Ok(data) => data.execution_cost_usdc,
            Err(e) => {
                warn!("Failed to record swap cost metrics: {e}");
                0.0 // Default to 0 USD
            },
        };

        responses.push(SwapImmediateResponse {
            quote: outcome.quote.into(),
            tx_hash: format!("{:#x}", outcome.tx_hash),
            execution_cost,
        });
    }

    Ok(warp::reply::json(&responses))
}
