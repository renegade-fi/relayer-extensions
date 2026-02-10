//! The RPC shim handler

use std::sync::Arc;

use renegade_types_core::Chain;
use warp::reply::Json;

use crate::{cli::Environment, custody_client::rpc_shim::JsonRpcRequest, server::Server};

/// Handler for the RPC shim
pub(crate) async fn rpc_handler(
    req: JsonRpcRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // TODO: Have chain-agnostic hedging client subsume this
    let chain = match server.environment {
        Environment::Mainnet => Chain::ArbitrumOne,
        Environment::Testnet => Chain::ArbitrumSepolia,
    };

    let custody_client = server.get_custody_client(&chain)?;
    let rpc_response = custody_client.handle_rpc_request(req).await;

    Ok(warp::reply::json(&rpc_response))
}
