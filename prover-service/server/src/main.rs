//! A service for generating proofs of Renegade circuits

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::unused_async)]

use std::net::SocketAddr;

use clap::Parser;
use http::StatusCode;
use tracing::{error, info, info_span};
use warp::{
    Filter,
    filters::body::BodyDeserializeError,
    reject::Rejection,
    reply::{Json, Reply, WithStatus},
};

use crate::{
    cli::Cli,
    error::{ProverServiceError, json_error},
    prover::{
        handle_link_commitments_reblind, handle_valid_commitments, handle_valid_fee_redemption,
        handle_valid_malleable_match_settle_atomic, handle_valid_match_settle,
        handle_valid_match_settle_atomic, handle_valid_offline_fee_settlement,
        handle_valid_reblind, handle_valid_wallet_create, handle_valid_wallet_update,
    },
};

mod cli;
mod error;
mod prover;

/// Entrypoint
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    cli.configure_telemetry().expect("failed to setup telemetry");

    // Run the server
    let routes = setup_routes();
    let listen_addr: SocketAddr = ([0, 0, 0, 0], cli.port).into();
    info!("listening on {}", listen_addr);
    warp::serve(routes).bind(listen_addr).await;
}

// --- Routes --- //

/// Setup the HTTP routes
fn setup_routes() -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    // Ping route
    let ping = warp::path("ping")
        .and(warp::get())
        .map(|| warp::reply::with_status("PONG", StatusCode::OK));

    // Prove valid wallet create
    let valid_wallet_create = warp::path("prove-valid-wallet-create")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_wallet_create);

    // Prove valid wallet update
    let valid_wallet_update = warp::path("prove-valid-wallet-update")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_wallet_update);

    // Prove valid commitments
    let valid_commitments = warp::path("prove-valid-commitments")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_commitments);

    // Prove valid reblind
    let valid_reblind = warp::path("prove-valid-reblind")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_reblind);

    let link_commitments_reblind = warp::path("link-commitments-reblind")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_link_commitments_reblind);

    // Prove valid match settle
    let valid_match_settle = warp::path("prove-valid-match-settle")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_match_settle);

    // Prove valid match settle atomic
    let valid_match_settle_atomic = warp::path("prove-valid-match-settle-atomic")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_match_settle_atomic);

    // Prove valid malleable match settle atomic
    let valid_malleable_match_settle_atomic =
        warp::path("prove-valid-malleable-match-settle-atomic")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(handle_valid_malleable_match_settle_atomic);

    // Prove valid fee redemption
    let valid_fee_redemption = warp::path("prove-valid-fee-redemption")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_fee_redemption);

    // Prove valid offline fee settlement
    let valid_offline_fee_settlement = warp::path("prove-valid-offline-fee-settlement")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_valid_offline_fee_settlement);

    ping.or(valid_wallet_create)
        .or(valid_wallet_update)
        .or(valid_commitments)
        .or(valid_reblind)
        .or(link_commitments_reblind)
        .or(valid_match_settle)
        .or(valid_match_settle_atomic)
        .or(valid_malleable_match_settle_atomic)
        .or(valid_fee_redemption)
        .or(valid_offline_fee_settlement)
        .with(with_tracing())
        .recover(handle_rejection)
}

// --- Middleware --- //

/// Custom tracing filter that creates spans for requests at info level
/// with the prover_service::request target to work with our RUST_LOG
/// configuration
fn with_tracing() -> warp::trace::Trace<impl Fn(warp::trace::Info) -> tracing::Span + Clone> {
    warp::trace(|info| {
        let span = info_span!(
            target: "prover_service::request",
            "handle_request",
            method = %info.method(),
            path = %info.path(),
        );

        span
    })
}

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: Rejection) -> Result<WithStatus<Json>, Rejection> {
    let reply = if let Some(api_error) = err.find::<ProverServiceError>() {
        api_error.to_reply()
    } else if let Some(err) = err.find::<BodyDeserializeError>() {
        json_error(&err.to_string(), StatusCode::BAD_REQUEST)
    } else if err.is_not_found() {
        json_error("Not Found", StatusCode::NOT_FOUND)
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        json_error("Method Not Allowed", StatusCode::METHOD_NOT_ALLOWED)
    } else {
        error!("unhandled rejection: {:?}", err);
        json_error("Internal Server Error", StatusCode::INTERNAL_SERVER_ERROR)
    };

    Ok(reply)
}
