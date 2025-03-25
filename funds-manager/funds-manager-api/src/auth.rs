//! Auth helpers for the API
use crate::types::quoters::ExecutionQuote;
use hex;
use hmac::{Hmac, Mac};
use http::HeaderMap;
use itertools::Itertools;
use sha2::Sha256;

/// The header key for the HMAC signature
pub const X_SIGNATURE_HEADER: &str = "X-Signature";
/// The prefix for Renegade headers, these headers are included in the HMAC
/// signature
pub const RENEGADE_HEADER_PREFIX: &str = "x-renegade-";

/// Compute an hmac for the given request
pub fn compute_hmac(
    hmac_key: &[u8],
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Vec<u8> {
    // Construct the MAC
    let mut mac = Hmac::<Sha256>::new_from_slice(hmac_key).expect("HMAC error");

    // Update with method, path, headers and body in order
    mac.update(method.as_bytes());
    mac.update(path.as_bytes());
    add_headers_to_hmac(&mut mac, headers);
    mac.update(body);

    // Check the signature
    mac.finalize().into_bytes().to_vec()
}

/// Hash headers into an HMAC
fn add_headers_to_hmac(mac: &mut Hmac<Sha256>, headers: &HeaderMap) {
    let mut renegade_headers = headers
        .iter()
        .filter_map(|(k, v)| {
            let key = k.to_string().to_lowercase();
            if key.starts_with(RENEGADE_HEADER_PREFIX) {
                Some((key, v))
            } else {
                None
            }
        })
        .collect_vec();
    renegade_headers.sort_by(|a, b| a.0.cmp(&b.0));

    for (key, value) in renegade_headers {
        mac.update(key.as_bytes());
        mac.update(value.as_bytes());
    }
}

/// Compute an HMAC signature for an execution quote
pub fn compute_quote_hmac(key: &[u8], quote: &ExecutionQuote) -> String {
    // Create a canonical string representation of the quote
    let canonical = format!(
        "{}{}{}{}{}{}{}{}{}{}",
        quote.buy_token_address,
        quote.sell_token_address,
        quote.sell_amount,
        quote.buy_amount,
        quote.from,
        quote.to,
        hex::encode(&quote.data),
        quote.value,
        quote.gas_price,
        quote.estimated_gas
    );

    // Compute HMAC
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC error");
    mac.update(canonical.as_bytes());
    let result = mac.finalize();

    // Convert to hex string
    hex::encode(result.into_bytes())
}
