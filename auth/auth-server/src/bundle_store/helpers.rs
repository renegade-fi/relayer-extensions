//! Helper methods for the bundle store
use alloy_primitives::{hex, keccak256};
use renegade_api::http::external_match::{ApiBoundedMatchResult, ApiExternalMatchResult};
use renegade_circuit_types::wallet::Nullifier;
use renegade_constants::Scalar;

use crate::error::AuthServerError;

/// Generates a deterministic bundle ID by hashing together the nullifier
/// and the match amounts.
///
/// This approach is prone to collisions because there is no single unique
/// customer identifier shared by both the on‑chain listener and the HTTP
/// handler. Without a common customer identifier present in both domains that
/// can be incorporated into the hash, it is possible for different customers to
/// produce the same bundle ID.
///
/// Collisions have the following consequences, which are deemed acceptable:
/// - Same‑customer collision: Metrics and rate‑limit accounting remain correct
///   as the collision occurs within the same customer context.
/// - Different‑customer collision: Metrics or rate‑limit allowance may be
///   attributed to the wrong customer.
///
/// We could use the proof stored in calldata to uniquely identify each bundle,
/// but this approach would break as soon as a bundle cache is introduced.
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

/// Generates a deterministic bundle ID for a malleable match
///
/// See the disclaimers in `generate_bundle_id` for more details.
pub fn generate_malleable_bundle_id(
    match_result: &ApiBoundedMatchResult,
    nullifier: &Nullifier,
) -> Result<String, AuthServerError> {
    let mut bytes = serde_json::to_vec(match_result).map_err(AuthServerError::serde)?;
    bytes.extend(nullifier.to_bytes_be());
    Ok(hex::encode(keccak256::<&[u8]>(&bytes)))
}
