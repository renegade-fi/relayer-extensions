//! SQS message type definitions for the darkpool indexer

use renegade_constants::Scalar;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The top-level enum of all possible SQS messages
#[derive(Serialize, Deserialize)]
pub enum SqsMessage {
    /// A message representing the registration of a new master view seed
    RegisterMasterViewSeed(MasterViewSeedMessage),
    /// A message representing the spending of a state object's nullifier
    /// onchain
    NullifierSpend(NullifierSpendMessage),
}

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

/// A message representing the spending of a state object's nullifier onchain
#[derive(Serialize, Deserialize)]
pub struct NullifierSpendMessage {
    /// The nullifier that was spent
    pub nullifier: Scalar,
    /// The new public shares of the state object
    pub public_shares: Vec<Scalar>,
    /// The new recovery ID of the state object
    pub recovery_id: Scalar,
    /// The new version of the state object
    pub version: usize,
    /// The type of the state object
    pub object_type: ApiStateObjectType,
    /// The block number at which the nullifier was spent
    pub block_number: u64,
}

/// The type of a state object
#[derive(Serialize, Deserialize)]
pub enum ApiStateObjectType {
    /// An intent state object
    Intent,
    /// A balance state object
    Balance,
}
