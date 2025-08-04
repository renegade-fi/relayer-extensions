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
    reject::Rejection,
    reply::{Json, Reply, WithStatus},
};

use crate::{
    cli::Cli,
    error::{ProverServiceError, json_error},
};

mod cli;
mod error;

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

    ping.with(with_tracing()).recover(handle_rejection)
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
