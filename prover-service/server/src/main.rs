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
use tracing::info;
use warp::{Filter, reject::Rejection, reply::Reply};

use crate::{
    cli::Cli,
    middleware::{basic_auth, handle_rejection, with_tracing},
    prover::{
        handle_link_commitments_reblind, handle_valid_commitments, handle_valid_fee_redemption,
        handle_valid_malleable_match_settle_atomic, handle_valid_match_settle,
        handle_valid_match_settle_atomic, handle_valid_offline_fee_settlement,
        handle_valid_reblind, handle_valid_wallet_create, handle_valid_wallet_update,
    },
};

mod cli;
mod error;
mod middleware;
mod prover;

/// The runtime stack size to use for the server
const RUNTIME_STACK_SIZE: usize = 50 * 1024 * 1024; // 50MB

/// Entrypoint
fn main() {
    // Create a custom tokio runtime with 50MB stack size
    // The warp filters sometimes overflow the stack in debug mode; so we manually
    // setup the stack
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(RUNTIME_STACK_SIZE)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    runtime.block_on(async_main());
}

/// Async main function
async fn async_main() {
    let cli = Cli::parse();
    cli.configure_telemetry().expect("failed to setup telemetry");

    // Run the server
    let routes = setup_routes(cli.auth_password);
    let listen_addr: SocketAddr = ([0, 0, 0, 0], cli.port).into();
    info!("listening on {}", listen_addr);
    warp::serve(routes).bind(listen_addr).await;
}

// --- Routes --- //

/// Setup the HTTP routes
fn setup_routes(
    auth_pwd: String,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    // Ping route
    let ping = warp::path("ping")
        .and(warp::get())
        .map(|| warp::reply::with_status("PONG", StatusCode::OK));

    // Prove valid wallet create
    let valid_wallet_create = warp::path("prove-valid-wallet-create")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_wallet_create);

    // Prove valid wallet update
    let valid_wallet_update = warp::path("prove-valid-wallet-update")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_wallet_update);

    // Prove valid commitments
    let valid_commitments = warp::path("prove-valid-commitments")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_commitments);

    // Prove valid reblind
    let valid_reblind = warp::path("prove-valid-reblind")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_reblind);

    let link_commitments_reblind = warp::path("link-commitments-reblind")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_link_commitments_reblind);

    // Prove valid match settle
    let valid_match_settle = warp::path("prove-valid-match-settle")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_match_settle);

    // Prove valid match settle atomic
    let valid_match_settle_atomic = warp::path("prove-valid-match-settle-atomic")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_match_settle_atomic);

    // Prove valid malleable match settle atomic
    let valid_malleable_match_settle_atomic =
        warp::path("prove-valid-malleable-match-settle-atomic")
            .and(warp::post())
            .and(basic_auth(auth_pwd.clone()))
            .and(warp::body::json())
            .and_then(handle_valid_malleable_match_settle_atomic);

    // Prove valid fee redemption
    let valid_fee_redemption = warp::path("prove-valid-fee-redemption")
        .and(warp::post())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_fee_redemption);

    // Prove valid offline fee settlement
    let valid_offline_fee_settlement = warp::path("prove-valid-offline-fee-settlement")
        .and(warp::post())
        .and(basic_auth(auth_pwd))
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
