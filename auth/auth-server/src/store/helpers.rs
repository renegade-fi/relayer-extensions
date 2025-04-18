use alloy_primitives::{hex, keccak256};
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_circuit_types::wallet::Nullifier;
use renegade_constants::Scalar;

use crate::error::AuthServerError;

/// Generates a bundle ID from the match bundle
pub fn generate_bundle_id(
    match_result: &ApiExternalMatchResult,
    nullifier: &Nullifier,
) -> Result<String, AuthServerError> {
    let quote_amt = match_result.quote_amount;
    let base_amt = match_result.base_amount;

    let mut bytes = nullifier.to_bytes_be();
    bytes.extend(Scalar::from(quote_amt).to_bytes_be());
    bytes.extend(Scalar::from(base_amt).to_bytes_be());
    Ok(hex::encode(keccak256::<&[u8]>(&bytes)))
}
