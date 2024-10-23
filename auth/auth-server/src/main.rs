//! The relayer authentication server
//!
//! This server is run independently of the relayer and is responsible for
//! issuing and managing API keys that provide access to the relayer's API.
//!
//! As such, the server holds the relayer admin key, and proxies authenticated
//! requests to the relayer directly
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

pub(crate) mod error;
pub(crate) mod models;
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub(crate) mod schema;
mod server;

use auth_server_api::{API_KEYS_PATH, DEACTIVATE_API_KEY_PATH};
use clap::Parser;
use renegade_util::telemetry::configure_telemetry;
use reqwest::StatusCode;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tracing::{error, info};
use uuid::Uuid;
use warp::{Filter, Rejection, Reply};

use server::Server;

/// The default internal server error message
const DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal Server Error";

// -------
// | CLI |
// -------

/// The command line arguments for the auth server
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// The database url
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,
    /// The encryption key used to encrypt/decrypt database values
    #[arg(long, env = "ENCRYPTION_KEY")]
    pub encryption_key: String,
    /// The URL of the relayer
    #[arg(long, env = "RELAYER_URL")]
    pub relayer_url: String,
    /// The admin key for the relayer
    #[arg(long, env = "RELAYER_ADMIN_KEY")]
    pub relayer_admin_key: String,
    /// The port to run the server on
    #[arg(long, env = "PORT", default_value = "3000")]
    pub port: u16,
    /// Whether to enable datadog logging
    #[arg(long)]
    pub datadog_logging: bool,
}

// -------------
// | Api Types |
// -------------

/// Custom error type for API errors
#[derive(Error, Debug)]
pub enum ApiError {
    /// An internal server error
    #[error("Internal server error: {0}")]
    InternalError(String),
    /// A bad request error
    #[error("Bad request: {0}")]
    BadRequest(String),
    /// An unauthorized error
    #[error("Unauthorized")]
    Unauthorized,
}

// Implement warp::reject::Reject for ApiError
impl warp::reject::Reject for ApiError {}

// ----------
// | Server |
// ----------

/// The main function for the auth server
#[tokio::main]
async fn main() {
    let args = Cli::parse();
    let listen_addr: SocketAddr = ([0, 0, 0, 0], args.port).into();

    // Setup logging
    configure_telemetry(
        args.datadog_logging, // datadog_enabled
        false,                // otlp_enabled
        false,                // metrics_enabled
        "".to_string(),       // collector_endpoint
        "",                   // statsd_host
        0,                    // statsd_port
    )
    .expect("failed to setup telemetry");

    // Create the server
    let server = Server::new(args).await.expect("Failed to create server");
    let server = Arc::new(server);

    // --- Routes --- //

    // Ping route
    let ping = warp::path("ping")
        .and(warp::get())
        .map(|| warp::reply::with_status("PONG", StatusCode::OK));

    // Add an API key
    let add_api_key = warp::path(API_KEYS_PATH)
        .and(warp::post())
        .and(warp::body::json())
        .and(with_server(server.clone()))
        .and_then(|request, server: Arc<Server>| async move { server.add_key(request).await });

    // Expire an API key
    let expire_api_key = warp::path(API_KEYS_PATH)
        .and(warp::path::param::<Uuid>())
        .and(warp::path(DEACTIVATE_API_KEY_PATH))
        .and(warp::post())
        .and(with_server(server.clone()))
        .and_then(|id: Uuid, server: Arc<Server>| async move { server.expire_key(id).await });

    // Proxy route
    let proxy = warp::path::full()
        .and(warp::method())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, method, headers, body, server: Arc<Server>| async move {
            server.handle_proxy_request(path, method, headers, body).await
        });

    // Bind the server and listen
    info!("Starting auth server on port {}", listen_addr.port());
    let routes = ping.or(add_api_key).or(expire_api_key).or(proxy).recover(handle_rejection);
    warp::serve(routes).bind(listen_addr).await;
}

/// Helper function to pass the server to filters
fn with_server(
    server: Arc<Server>,
) -> impl Filter<Extract = (Arc<Server>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
}

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: Rejection) -> Result<impl Reply, Rejection> {
    if let Some(api_error) = err.find::<ApiError>() {
        let (code, message) = match api_error {
            ApiError::InternalError(e) => {
                error!("Internal server error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE)
            },
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.as_str()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
        };

        Ok(json_error(message, code))
    } else if err.is_not_found() {
        Ok(json_error("Not Found", StatusCode::NOT_FOUND))
    } else {
        error!("unhandled rejection: {:?}", err);
        Ok(json_error("Internal Server Error", StatusCode::INTERNAL_SERVER_ERROR))
    }
}

// -----------
// | Helpers |
// -----------

/// Return a json error from a string message
fn json_error(msg: &str, code: StatusCode) -> impl Reply {
    let json = json!({ "error": msg });
    warp::reply::with_status(warp::reply::json(&json), code)
}
