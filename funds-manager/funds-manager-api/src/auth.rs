//! Auth helpers for the API
use http::HeaderMap;
use itertools::Itertools;

/// The header key for the HMAC signature
pub const X_SIGNATURE_HEADER: &str = "X-Signature";
/// The prefix for Renegade headers, these headers are included in the HMAC
/// signature
pub const RENEGADE_HEADER_PREFIX: &str = "x-renegade-";

/// Get the bytes for the given request for HMAC computation
pub fn get_request_bytes(method: &str, path: &str, headers: &HeaderMap, body: &[u8]) -> Vec<u8> {
    // Build the message to sign
    let mut message = Vec::new();
    message.extend_from_slice(method.as_bytes());
    message.extend_from_slice(path.as_bytes());
    message.extend(get_header_bytes(headers));
    message.extend_from_slice(body);

    message
}

/// Get bytes from headers for HMAC computation
fn get_header_bytes(headers: &HeaderMap) -> Vec<u8> {
    let mut renegade_headers = headers
        .iter()
        .filter_map(|(k, v)| {
            let key = k.to_string().to_lowercase();
            if key.starts_with(RENEGADE_HEADER_PREFIX) { Some((key, v)) } else { None }
        })
        .collect_vec();
    renegade_headers.sort_by(|a, b| a.0.cmp(&b.0));

    let mut header_bytes = Vec::new();
    for (key, value) in renegade_headers {
        header_bytes.extend_from_slice(key.as_bytes());
        header_bytes.extend_from_slice(value.as_bytes());
    }
    header_bytes
}
