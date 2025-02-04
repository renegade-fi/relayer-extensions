//! API types for the auth server

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

use alloy_primitives::Address;
use renegade_api::http::external_match::AtomicMatchApiBundle;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The Renegade API key header
pub const RENEGADE_API_KEY_HEADER: &str = "X-Renegade-Api-Key";

// ----------------------
// | API Key Management |
// ----------------------

/// The path to create a new API key
///
/// POST /api-keys
pub const API_KEYS_PATH: &str = "api-keys";
/// The path to mark an API key as inactive
///
/// POST /api-keys/{id}/deactivate
pub const DEACTIVATE_API_KEY_PATH: &str = "/api-keys/{id}/deactivate";

/// A request to create a new API key
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateApiKeyRequest {
    /// The API key id
    pub id: Uuid,
    /// The API key secret
    ///
    /// Expected as a base64 encoded string
    pub secret: String,
    /// A description of the API key's purpose
    pub description: String,
}

/// A sponsored match response from the auth server
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SponsoredMatchResponse {
    /// The external match bundle
    pub match_bundle: AtomicMatchApiBundle,
    /// Whether or not the match was sponsored
    pub is_sponsored: bool,
}

/// The query parameters used for gas sponsorship
#[derive(Debug, Serialize, Deserialize)]
pub struct GasSponsorshipQueryParams {
    /// Whether to use gas sponsorship for the external match
    pub use_gas_sponsorship: Option<bool>,
    /// The address to refund gas to
    pub refund_address: Option<String>,
}

impl GasSponsorshipQueryParams {
    /// Get the refund address, or the default zero address if not provided
    pub fn get_refund_address(&self) -> Result<Address, String> {
        self.refund_address
            .as_ref()
            .map(|s| s.parse())
            .unwrap_or(Ok(Address::ZERO))
            .map_err(|_| "invalid refund address".to_string())
    }
}
