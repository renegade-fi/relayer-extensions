//! HTTP server executor and route building

use std::sync::Arc;

use auth_server_api::API_KEYS_PATH;
use reqwest::StatusCode;
use serde_json::json;
use tracing::{error, info, info_span};
use uuid::Uuid;
use warp::{
    Filter, Rejection,
    reply::{Json, WithStatus},
};

use crate::{ApiError, error::AuthServerError};

use super::{Server, worker::HttpServerConfig};

/// The executor that runs in a thread
#[derive(Clone)]
pub struct HttpServerExecutor {
    /// The configuration for the HTTP server
    pub(crate) config: HttpServerConfig,
}

impl HttpServerExecutor {
    /// Create a new HTTP server executor
    pub fn new(config: HttpServerConfig) -> Self {
        Self { config }
    }

    /// The main execution loop for the HTTP server
    pub async fn execute(self) {
        let listen_addr = self.config.listen_addr;

        // Setup the server
        let server = Server::setup(
            self.config.args,
            self.config.gas_sponsor_address,
            self.config.malleable_match_connector_address,
            self.config.bundle_store,
            self.config.rate_limiter,
            self.config.price_reporter_client,
            self.config.gas_cost_sampler,
        )
        .await
        .expect("Failed to create server");
        let server = Arc::new(server);

        let routes = Self::build_routes(server);

        info!("Starting auth server on port {}", listen_addr.port());
        warp::serve(routes).bind(listen_addr).await;
    }

    /// Build all the routes for the server
    pub fn build_routes(
        server: Arc<Server>,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        // --- Management Routes --- //

        // Ping route
        let ping = warp::path("ping")
            .and(warp::get())
            .map(|| warp::reply::with_status("PONG", StatusCode::OK));

        // Get all API keys
        let get_all_keys = warp::path(API_KEYS_PATH)
            .and(warp::path::end())
            .and(warp::get())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(with_server(server.clone()))
            .and_then(|path, headers, server: Arc<Server>| async move {
                server.get_all_keys(path, headers).await
            });

        // Add an API key
        let add_api_key = warp::path(API_KEYS_PATH)
            .and(warp::path::end())
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, server: Arc<Server>| async move {
                server.add_key(path, headers, body).await
            });

        // Expire an API key
        let expire_api_key = warp::path(API_KEYS_PATH)
            .and(warp::path::param::<Uuid>())
            .and(warp::path("deactivate"))
            .and(warp::path::end())
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|id, path, headers, body, server: Arc<Server>| async move {
                server.expire_key(id, path, headers, body).await
            });

        // Whitelist an API key
        let whitelist_api_key = warp::path(API_KEYS_PATH)
            .and(warp::path::param::<Uuid>())
            .and(warp::path("whitelist"))
            .and(warp::path::end())
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|id, path, headers, body, server: Arc<Server>| async move {
                server.whitelist_api_key(id, path, headers, body).await
            });

        // Remove a whitelist entry for an API key
        let remove_whitelist_entry = warp::path(API_KEYS_PATH)
            .and(warp::path::param::<Uuid>())
            .and(warp::path("remove-whitelist"))
            .and(warp::path::end())
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|id, path, headers, body, server: Arc<Server>| async move {
                server.remove_whitelist_entry(id, path, headers, body).await
            });

        // Get all user fees
        let get_all_user_fees = warp::path!("v0" / "fees" / "get-per-user-fees")
            .and(warp::get())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(with_server(server.clone()))
            .and_then(|path, headers, server: Arc<Server>| async move {
                server.get_all_user_fees(path, headers).await
            });

        // Set the default external match fee for an asset
        let set_asset_default_fee = warp::path!("v0" / "fees" / "set-asset-default-fee")
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, server: Arc<Server>| async move {
                server.set_asset_default_fee(path, headers, body).await
            });

        // Set the per-user fee override for an asset
        let set_user_fee_override = warp::path!("v0" / "fees" / "set-user-fee-override")
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, server: Arc<Server>| async move {
                server.set_user_fee_override(path, headers, body).await
            });

        // Remove the default external match fee for an asset
        let remove_asset_default_fee = warp::path!("v0" / "fees" / "remove-asset-default-fee")
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, server: Arc<Server>| async move {
                server.remove_asset_default_fee(path, headers, body).await
            });

        // Remove the per-user fee override for an asset
        let remove_user_fee_override = warp::path!("v0" / "fees" / "remove-user-fee-override")
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, server: Arc<Server>| async move {
                server.remove_user_fee_override(path, headers, body).await
            });

        // --- Proxied Routes --- //

        let external_quote_path = warp::path("v0")
            .and(warp::path("matching-engine"))
            .and(warp::path("quote"))
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_query_string())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
                server.handle_quote_request(path, headers, body, query_str).await
            });

        let external_quote_assembly_path = warp::path("v0")
            .and(warp::path("matching-engine"))
            .and(warp::path("assemble-external-match"))
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_query_string())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
                server.handle_assemble_quote_request(path, headers, body, query_str).await
            });

        let external_malleable_assembly_path = warp::path("v0")
            .and(warp::path("matching-engine"))
            .and(warp::path("assemble-malleable-external-match"))
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_query_string())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
                server.handle_assemble_malleable_quote_request(path, headers, body, query_str).await
            });

        let atomic_match_path = warp::path("v0")
            .and(warp::path("matching-engine"))
            .and(warp::path("request-external-match"))
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_query_string())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
                server.handle_external_match_request(path, headers, body, query_str).await
            });

        let order_book_depth_with_mint = warp::path("v0")
            .and(warp::path("order_book"))
            .and(warp::path("depth"))
            .and(warp::path::param::<String>())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(with_server(server.clone()))
            .and_then(|_mint, path, headers, server: Arc<Server>| async move {
                server.handle_order_book_request(path, headers).await
            });

        let order_book_depth = warp::path("v0")
            .and(warp::path("order_book"))
            .and(warp::path("depth"))
            .and(warp::path::end())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(with_server(server.clone()))
            .and_then(|path, headers, server: Arc<Server>| async move {
                server.handle_order_book_request(path, headers).await
            });

        let rfqt_levels_path = warp::path!("rfqt" / "v3" / "levels")
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(with_query_string())
            .and(with_server(server.clone()))
            .and_then(|path, headers, query_str, server: Arc<Server>| async move {
                server.handle_rfqt_levels_request(path, headers, query_str).await
            });

        let rfqt_quote_path = warp::path!("rfqt" / "v3" / "quote")
            .and(warp::post())
            .and(warp::path::full())
            .and(warp::header::headers_cloned())
            .and(warp::body::bytes())
            .and(with_query_string())
            .and(with_server(server.clone()))
            .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
                server.handle_rfqt_quote_request(path, headers, body, query_str).await
            });

        // Combine all routes
        ping.or(atomic_match_path)
            .or(external_quote_path)
            .or(external_quote_assembly_path)
            .or(external_malleable_assembly_path)
            .or(expire_api_key)
            .or(whitelist_api_key)
            .or(remove_whitelist_entry)
            .or(add_api_key)
            .or(get_all_keys)
            .or(get_all_user_fees)
            .or(set_asset_default_fee)
            .or(set_user_fee_override)
            .or(remove_asset_default_fee)
            .or(remove_user_fee_override)
            .or(order_book_depth_with_mint)
            .or(order_book_depth)
            .or(rfqt_levels_path)
            .or(rfqt_quote_path)
            .boxed()
            .with(with_tracing())
            .recover(handle_rejection)
    }
}

/// Helper function to pass the server to filters
fn with_server(
    server: Arc<Server>,
) -> impl Filter<Extract = (Arc<Server>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
}

/// Helper function to parse the raw query string, returning an empty string
/// instead of rejecting in the case that no query string is present
fn with_query_string() -> impl Filter<Extract = (String,), Error = std::convert::Infallible> + Clone
{
    warp::query::raw().or_else(|_| async { Ok((String::new(),)) })
}

/// Custom tracing filter that creates spans for requests at info level
/// with the auth_server::request target to work with our RUST_LOG configuration
fn with_tracing() -> warp::trace::Trace<impl Fn(warp::trace::Info) -> tracing::Span + Clone> {
    warp::trace(|info| {
        let span = info_span!(
            target: "auth_server::request",
            "handle_request",
            method = %info.method(),
            path = %info.path(),
        );

        span
    })
}

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: Rejection) -> Result<WithStatus<Json>, Rejection> {
    let reply = if let Some(api_error) = err.find::<ApiError>() {
        api_error_to_reply(api_error)
    } else if let Some(auth_error) = err.find::<AuthServerError>().cloned() {
        let api_err = ApiError::from(auth_error);
        api_error_to_reply(&api_err)
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

/// Convert an `ApiError` into a reply
fn api_error_to_reply(api_error: &ApiError) -> WithStatus<Json> {
    const DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal Server Error";
    let (code, message) = match api_error {
        ApiError::InternalError(e) => {
            error!("Internal server error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE)
        },
        ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.as_str()),
        ApiError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded"),
        ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
    };

    json_error(message, code)
}

/// Return a json error from a string message
fn json_error(msg: &str, code: StatusCode) -> WithStatus<Json> {
    let json = json!({ "error": msg });
    warp::reply::with_status(warp::reply::json(&json), code)
}
