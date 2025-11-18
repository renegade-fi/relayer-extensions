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
    /// A message representing the registration of a new recovery ID
    RegisterRecoveryId(RecoveryIdMessage),
    /// A message representing the spending of a state object's nullifier
    /// onchain
    NullifierSpend(NullifierSpendMessage),
    /// A message representing the creation of a new public intent
    CreatePublicIntent(CreatePublicIntentMessage),
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

/// A message representing the registration of a new recovery ID
#[derive(Serialize, Deserialize)]
pub struct RecoveryIdMessage {
    /// The recovery ID that was registered
    pub recovery_id: Scalar,
    /// The transaction hash of the recovery ID registration
    pub tx_hash: TxHash,
}

/// A message representing the spending of a state object's nullifier onchain
#[derive(Serialize, Deserialize)]
pub struct NullifierSpendMessage {
    /// The nullifier that was spent
    pub nullifier: Scalar,
    /// The transaction hash of the nullifier spend
    pub tx_hash: TxHash,
}

/// A message representing the creation of a new public intent
#[derive(Serialize, Deserialize)]
pub struct CreatePublicIntentMessage {
    /// The intent hash
    pub intent_hash: String,
    /// The address of the intent's owner
    pub owner_address: Address,
    /// The transaction hash of the public intent creation
    pub tx_hash: TxHash,
}
