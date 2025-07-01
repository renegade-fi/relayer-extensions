//! Key management API endpoints

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An API key entry
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKey {
    /// The API key id
    pub id: Uuid,
    /// A description of the API key's purpose
    pub description: String,
    /// Whether the API key is active
    pub is_active: bool,
    /// Whether the API key is whitelisted for external match flow rate limiting
    pub rate_limit_whitelisted: bool,
    /// The date and time the API key was created
    ///
    /// In seconds since epoch
    pub created_at: u64,
}

/// A response containing all API keys
#[derive(Debug, Serialize, Deserialize)]
pub struct AllKeysResponse {
    /// The list of API keys
    pub keys: Vec<ApiKey>,
}
