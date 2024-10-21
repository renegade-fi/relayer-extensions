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

use bytes::Bytes;
use clap::Parser;
use reqwest::{Client, Method, StatusCode};
use serde_json::json;
use std::net::SocketAddr;
use thiserror::Error;
use tracing::{error, info};
use warp::{Filter, Rejection, Reply};

// -------
// | CLI |
// -------

/// The command line arguments for the auth server
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The URL of the relayer
    #[arg(long, env = "RELAYER_URL")]
    relayer_url: String,
    /// The admin key for the relayer
    #[arg(long, env = "RELAYER_ADMIN_KEY")]
    relayer_admin_key: String,
    /// The port to run the server on
    #[arg(long, env = "PORT", default_value = "3030")]
    port: u16,
    /// Whether to enable datadog logging
    #[arg(long)]
    datadog_logging: bool,
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
}

// Implement warp::reject::Reject for ApiError
impl warp::reject::Reject for ApiError {}

// ----------
// | Server |
// ----------

/// The main function for the auth server
#[tokio::main]
async fn main() {
    let args = Args::parse();
    let listen_addr: SocketAddr = ([0, 0, 0, 0], args.port).into();

    // TODO: Setup logging

    // --- Routes --- //

    // Ping route
    let ping = warp::path("ping")
        .and(warp::get())
        .map(|| warp::reply::with_status("PONG", StatusCode::OK));

    // Proxy route
    let proxy = warp::path::full()
        .and(warp::method())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(warp::any().map(move || args.relayer_url.clone()))
        .and(warp::any().map(move || args.relayer_admin_key.clone()))
        .and_then(handle_request);

    // Bind the server and listen
    info!("Starting auth server on port {}", args.port);
    let routes = ping.or(proxy).recover(handle_rejection);
    warp::serve(routes).bind(listen_addr).await;
}

/// Handle a request to the relayer
async fn handle_request(
    path: warp::path::FullPath,
    method: Method,
    headers: warp::hyper::HeaderMap,
    body: Bytes,
    relayer_url: String,
    relayer_admin_key: String,
) -> Result<impl Reply, Rejection> {
    let client = Client::new();
    let url = format!("{}{}", relayer_url, path.as_str());

    let mut req = client.request(method, &url).headers(headers).body(body);
    req = req.header("X-Admin-Key", &relayer_admin_key);

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp.bytes().await.map_err(|e| {
                warp::reject::custom(ApiError::InternalError(format!(
                    "Failed to read response body: {}",
                    e
                )))
            })?;

            let mut response = warp::http::Response::new(body);
            *response.status_mut() = status;
            *response.headers_mut() = headers;

            Ok(response)
        },
        Err(e) => {
            error!("Error proxying request: {}", e);
            Err(warp::reject::custom(ApiError::InternalError(e.to_string())))
        },
    }
}

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: Rejection) -> Result<impl Reply, Rejection> {
    if let Some(api_error) = err.find::<ApiError>() {
        let (code, message) = match api_error {
            ApiError::InternalError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
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
