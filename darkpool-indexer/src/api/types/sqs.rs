//! SQS message type definitions for the darkpool indexer

use renegade_constants::Scalar;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A message representing the registration of a new master view seed
#[derive(Serialize, Deserialize)]
pub struct MasterViewSeedMessage {
    /// The account ID of the seed owner
    pub account_id: Uuid,
    /// The address of the seed's owner
    pub owner_address: String,
    /// The master view seed
    pub seed: Scalar,
}
