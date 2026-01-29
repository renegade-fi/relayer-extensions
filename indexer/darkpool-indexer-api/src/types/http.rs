//! HTTP API type definitions for the darkpool indexer

use alloy_primitives::B256;
use renegade_circuit_types::Amount;
use renegade_darkpool_types::{
    balance::DarkpoolStateBalance,
    intent::{DarkpoolStateIntent, Intent},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------
// | Backfill Endpoint |
// ---------------------

/// A request to backfill a user's state
#[derive(Serialize, Deserialize)]
pub struct BackfillRequest {
    /// The account ID to backfill
    pub account_id: Uuid,
}

// --------------------------
// | Get User State Endpoint |
// --------------------------

/// A state object returned by the API
#[derive(Serialize, Deserialize)]
pub enum ApiStateObject {
    /// A balance state object
    Balance(ApiBalance),
    /// An intent state object
    Intent(ApiIntent),
    /// A public intent state object
    PublicIntent(ApiPublicIntent),
}

/// A balance state object returned by the API
#[derive(Serialize, Deserialize)]
pub struct ApiBalance {
    /// The underlying balance circuit type
    pub balance: DarkpoolStateBalance,
}

impl From<ApiBalance> for ApiStateObject {
    fn from(value: ApiBalance) -> Self {
        ApiStateObject::Balance(value)
    }
}

/// An intent state object returned by the API
#[derive(Serialize, Deserialize)]
pub struct ApiIntent {
    /// The underlying intent circuit type
    pub intent: DarkpoolStateIntent,
    /// The matching pool to which the intent is allocated
    pub matching_pool: String,
    /// Whether the intent allows external matches
    pub allow_external_matches: bool,
    /// The minimum fill size allowed for the intent
    pub min_fill_size: Amount,
    /// Whether to precompute a cancellation proof for the intent
    pub precompute_cancellation_proof: bool,
}

impl From<ApiIntent> for ApiStateObject {
    fn from(value: ApiIntent) -> Self {
        ApiStateObject::Intent(value)
    }
}

/// A public intent state object returned by the API
#[derive(Serialize, Deserialize)]
pub struct ApiPublicIntent {
    /// The intent's hash
    pub intent_hash: B256,
    /// The underlying intent circuit type
    pub intent: Intent,
    /// The matching pool to which the intent is allocated
    pub matching_pool: String,
    /// Whether the intent allows external matches
    pub allow_external_matches: bool,
    /// The minimum fill size allowed for the intent
    pub min_fill_size: Amount,
    /// Whether to precompute a cancellation proof for the intent
    pub precompute_cancellation_proof: bool,
}

impl From<ApiPublicIntent> for ApiStateObject {
    fn from(value: ApiPublicIntent) -> Self {
        ApiStateObject::PublicIntent(value)
    }
}

/// A response containing a user's active state objects
#[derive(Serialize, Deserialize)]
pub struct GetUserStateResponse {
    /// The list of active state objects
    pub active_state_objects: Vec<ApiStateObject>,
}
