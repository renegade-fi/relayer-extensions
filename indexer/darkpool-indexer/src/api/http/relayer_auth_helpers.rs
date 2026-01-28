//! HTTP auth helpers temporarily ported over from the relayer repo until all
//! crates build

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::{Engine, general_purpose as b64_general_purpose};
use itertools::Itertools;
use renegade_types_core::HmacKey;
use warp::http::HeaderMap;

// -------------
// | Constants |
// -------------

/// The number of bytes in an HMAC
const HMAC_LEN: usize = 32;

/// Header name for the HTTP auth signature; lower cased
const RENEGADE_AUTH_HEADER_NAME: &str = "x-renegade-auth";
/// Header name for the expiration timestamp of a signature; lower cased
const RENEGADE_SIG_EXPIRATION_HEADER_NAME: &str = "x-renegade-auth-expiration";
/// The header namespace to include in the HMAC
const RENEGADE_HEADER_NAMESPACE: &str = "x-renegade";

// ---------
// | Types |
// ---------

/// Error type for authentication helpers
#[derive(thiserror::Error, Debug)]
pub enum AuthError {
    /// Error displayed when the signature is invalid
    #[error("invalid signature")]
    InvalidSignature,
    /// Error displayed when the expiration format is invalid
    #[error("could not parse signature expiration timestamp")]
    ExpirationFormatInvalid,
    /// Error displayed when the HMAC is missing from the request
    #[error("HMAC is missing from the request")]
    HmacMissing,
    /// Error displayed when the HMAC format is invalid
    #[error("HMAC format invalid")]
    HmacFormatInvalid,
    /// Error displayed when the signature expiration header is missing
    #[error("signature expiration missing from headers")]
    SignatureExpirationMissing,
    /// Error displayed when a signature has expired
    #[error("signature expired")]
    SignatureExpired,
}

// -----------
// | Helpers |
// -----------

/// Validate a request signature with an expiration
pub fn validate_expiring_auth(
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
    key: &HmacKey,
) -> Result<(), AuthError> {
    // First check the expiration
    let expiration_ts = parse_auth_expiration_from_headers(headers)?;
    check_auth_timestamp(expiration_ts)?;

    // Then check the signature
    validate_auth(path, headers, body, key)
}

/// Validate a request signature without an expiration
pub fn validate_auth(
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
    key: &HmacKey,
) -> Result<(), AuthError> {
    // Parse the MAC from headers
    let mac = parse_hmac_from_headers(headers)?;

    // Compute the expected HMAC
    let expected_mac = create_request_signature(path, headers, body, key);
    if expected_mac != mac {
        return Err(AuthError::InvalidSignature);
    }

    Ok(())
}

/// Create a request signature
pub fn create_request_signature(
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
    key: &HmacKey,
) -> Vec<u8> {
    // Compute the expected HMAC
    let path_bytes = path.as_bytes();
    let header_bytes = get_header_bytes(headers);
    let payload = [path_bytes, &header_bytes, body].concat();

    key.compute_mac(&payload)
}

/// Parse an HMAC from headers
pub fn parse_hmac_from_headers(headers: &HeaderMap) -> Result<[u8; HMAC_LEN], AuthError> {
    let b64_hmac: &str = headers
        .get(RENEGADE_AUTH_HEADER_NAME)
        .ok_or(AuthError::HmacMissing)?
        .to_str()
        .map_err(|_| AuthError::HmacFormatInvalid)?;

    b64_general_purpose::STANDARD_NO_PAD
        .decode(b64_hmac)
        .map_err(|_| AuthError::HmacFormatInvalid)?
        .try_into()
        .map_err(|_| AuthError::HmacFormatInvalid)
}

/// Parse an expiration timestamp from headers
fn parse_auth_expiration_from_headers(headers: &HeaderMap) -> Result<u64, AuthError> {
    let sig_expiration = headers
        .get(RENEGADE_SIG_EXPIRATION_HEADER_NAME)
        .ok_or(AuthError::SignatureExpirationMissing)?;
    sig_expiration
        .to_str()
        .map_err(|_| AuthError::ExpirationFormatInvalid)
        .and_then(|s| s.parse::<u64>().map_err(|_| AuthError::ExpirationFormatInvalid))
}

/// Check a timestamp on a signature
fn check_auth_timestamp(expiration_ts: u64) -> Result<(), AuthError> {
    let now = SystemTime::now();
    let target_duration = Duration::from_millis(expiration_ts);
    let target_time = UNIX_EPOCH + target_duration;

    if now >= target_time {
        return Err(AuthError::SignatureExpired);
    }

    Ok(())
}

/// Get the header bytes to validate in an HMAC
fn get_header_bytes(headers: &HeaderMap) -> Vec<u8> {
    let mut headers_buf = Vec::new();

    // Filter out non-Renegade headers and the auth header
    let mut renegade_headers = headers
        .iter()
        .filter_map(|(k, v)| {
            let key = k.to_string().to_lowercase();
            if key.starts_with(RENEGADE_HEADER_NAMESPACE) && key != RENEGADE_AUTH_HEADER_NAME {
                Some((key, v))
            } else {
                None
            }
        })
        .collect_vec();

    // Sort alphabetically, then add to the buffer
    renegade_headers.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in renegade_headers {
        headers_buf.extend_from_slice(key.as_bytes());
        headers_buf.extend_from_slice(value.as_bytes());
    }

    headers_buf
}
