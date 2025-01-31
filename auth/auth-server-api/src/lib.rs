//! API types for the auth server

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

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

/// The query parameters used for gas sponsorship
#[derive(Debug, Serialize, Deserialize)]
pub struct GasSponsorshipQueryParams {
    /// Whether to use gas sponsorship for the external match
    pub use_gas_sponsorship: Option<bool>,
    /// The address to refund gas to
    pub refund_address: Option<String>,
}
