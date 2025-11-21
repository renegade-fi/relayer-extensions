//! HTTP API type definitions for the darkpool indexer

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A request to backfill a user's state
#[derive(Serialize, Deserialize)]
pub struct BackfillRequest {
    /// The account ID to backfill
    pub account_id: Uuid,
}
