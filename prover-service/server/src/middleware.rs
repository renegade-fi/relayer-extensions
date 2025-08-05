//! Authentication for the prover service

use http::StatusCode;
use http_auth_basic::Credentials;
use tracing::{error, info_span};
use warp::{
    Filter,
    filters::body::BodyDeserializeError,
    reject::{MissingHeader, Rejection},
    reply::{Json, WithStatus},
};

use crate::error::{ProverServiceError, json_error};

/// The auth header name
const HTTP_AUTH_HEADER: &str = "Authorization";
/// The HTTP basic auth username
const HTTP_AUTH_USERNAME: &str = "admin";

// -----------------
// | Authorization |
// -----------------

/// The HTTP basic auth implementation
pub(crate) fn basic_auth(pwd: String) -> impl Filter<Extract = (), Error = Rejection> + Clone {
    warp::header::<String>(HTTP_AUTH_HEADER)
        .and_then(move |auth: String| {
            let pwd = pwd.clone();
            async move { check_auth(auth, &pwd) }
        })
        .untuple_one()
}

/// Check authorization from a header
fn check_auth(header: String, pwd: &str) -> Result<(), Rejection> {
    let credential = Credentials::from_header(header).map_err(ProverServiceError::bad_request)?;

    if credential.user_id != HTTP_AUTH_USERNAME {
        return to_rejection(ProverServiceError::unauthorized("invalid username"));
    }
    if credential.password != pwd {
        return to_rejection(ProverServiceError::unauthorized("invalid password"));
    }
    Ok(())
}

/// Convert a prover service error into a rejection
///
/// Mostly used for ergonomic type annotation
fn to_rejection(err: ProverServiceError) -> Result<(), Rejection> {
    Err(err.into())
}

// ------------------
// | Error Handling |
// ------------------

/// Handle a rejection from an endpoint handler
pub(crate) async fn handle_rejection(err: Rejection) -> Result<WithStatus<Json>, Rejection> {
    let reply = if let Some(api_error) = err.find::<ProverServiceError>() {
        api_error.to_reply()
    } else if let Some(err) = err.find::<BodyDeserializeError>() {
        json_error(&err.to_string(), StatusCode::BAD_REQUEST)
    } else if let Some(err) = err.find::<MissingHeader>() {
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

// -----------
// | Tracing |
// -----------

/// Custom tracing filter that creates spans for requests at info level
/// with the prover_service::request target to work with our RUST_LOG
/// configuration
pub(crate) fn with_tracing()
-> warp::trace::Trace<impl Fn(warp::trace::Info) -> tracing::Span + Clone> {
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
