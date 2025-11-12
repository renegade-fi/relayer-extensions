//! SQS message type definitions for the darkpool indexer

use alloy_primitives::{Address, TxHash};
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
    pub owner_address: Address,
    /// The master view seed
    pub seed: Scalar,
}

/// A message representing the spending of a state object's nullifier onchain
#[derive(Serialize, Deserialize)]
pub struct NullifierSpendMessage {
    /// The nullifier that was spent
    pub nullifier: Scalar,
    /// The transaction hash of the nullifier spend
    pub tx_hash: TxHash,
}
