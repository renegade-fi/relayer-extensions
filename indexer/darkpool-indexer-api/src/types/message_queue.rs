//! Message type definitions for the darkpool indexer

use alloy_primitives::{Address, B256, TxHash};
use renegade_circuit_types::Amount;
use renegade_constants::Scalar;
use renegade_darkpool_types::intent::Intent;
use renegade_external_api::types::order::SignatureWithNonce;
use renegade_solidity_abi::v2::IDarkpoolV2::PublicIntentPermit;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The top-level enum of all possible message queue messages
#[derive(Serialize, Deserialize, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Message {
    /// A message representing the registration of a new master view seed
    RegisterMasterViewSeed(MasterViewSeedMessage),
    /// A message representing the registration of a new recovery ID
    RegisterRecoveryId(RecoveryIdMessage),
    /// A message representing the spending of a state object's nullifier
    /// onchain
    NullifierSpend(NullifierSpendMessage),
    /// A message representing the update of a public intent
    UpdatePublicIntent(UpdatePublicIntentMessage),
    /// A message representing the cancellation of a public intent
    CancelPublicIntent(CancelPublicIntentMessage),
    /// A message representing an update to a public intent's metadata
    UpdatePublicIntentMetadata(PublicIntentMetadataUpdateMessage),
}

impl Message {
    /// Derive the SQS deduplication ID for this message
    pub fn dedup_id(&self) -> String {
        match self {
            Message::RegisterMasterViewSeed(_) => Uuid::new_v4().to_string(),
            Message::RegisterRecoveryId(m) => m.recovery_id.to_string(),
            Message::NullifierSpend(m) => m.nullifier.to_string(),
            Message::UpdatePublicIntent(m) => m.tx_hash.to_string(),
            Message::CancelPublicIntent(m) => m.tx_hash.to_string(),
            Message::UpdatePublicIntentMetadata(_) => Uuid::new_v4().to_string(),
        }
    }

    /// Derive the SQS message group ID for this message
    pub fn message_group(&self) -> String {
        match self {
            Message::RegisterMasterViewSeed(m) => m.account_id.to_string(),
            Message::RegisterRecoveryId(m) => m.recovery_id.to_string(),
            Message::NullifierSpend(m) => m.nullifier.to_string(),
            Message::UpdatePublicIntent(m) => m.intent_hash.to_string(),
            Message::CancelPublicIntent(m) => m.intent_hash.to_string(),
            Message::UpdatePublicIntentMetadata(m) => m.intent_hash.to_string(),
        }
    }
}

/// A message representing the registration of a new master view seed
#[derive(Serialize, Deserialize, Clone)]
pub struct MasterViewSeedMessage {
    /// The account ID of the seed owner
    pub account_id: Uuid,
    /// The address of the seed's owner
    pub owner_address: Address,
    /// The master view seed
    pub seed: Scalar,
}

/// A message representing the registration of a new recovery ID
#[derive(Serialize, Deserialize, Clone)]
pub struct RecoveryIdMessage {
    /// The recovery ID that was registered
    pub recovery_id: Scalar,
    /// The transaction hash of the recovery ID registration
    pub tx_hash: TxHash,
    /// Whether this message originates from a backfill
    pub is_backfill: bool,
}

/// A message representing the spending of a state object's nullifier onchain
#[derive(Serialize, Deserialize, Clone)]
pub struct NullifierSpendMessage {
    /// The nullifier that was spent
    pub nullifier: Scalar,
    /// The transaction hash of the nullifier spend
    pub tx_hash: TxHash,
    /// Whether this message originates from a backfill
    pub is_backfill: bool,
}

/// A message representing the update of a public intent
#[derive(Serialize, Deserialize, Clone)]
pub struct UpdatePublicIntentMessage {
    /// The intent hash
    pub intent_hash: B256,
    /// The transaction hash of the public intent update
    pub tx_hash: TxHash,
    /// Whether this message originates from a backfill
    pub is_backfill: bool,
}

/// A message representing the cancellation of a public intent
#[derive(Serialize, Deserialize, Clone)]
pub struct CancelPublicIntentMessage {
    /// The intent hash
    pub intent_hash: B256,
    /// The transaction hash of the public intent cancellation
    pub tx_hash: TxHash,
    /// Whether this message originates from a backfill
    pub is_backfill: bool,
}

/// A message representing an update to a public intent's metadata
#[derive(Serialize, Deserialize, Clone)]
pub struct PublicIntentMetadataUpdateMessage {
    /// The intent hash
    pub intent_hash: B256,
    /// The public intent
    pub intent: Intent,
    /// The intent signature
    pub intent_signature: SignatureWithNonce,
    /// The permit for the intent
    pub permit: PublicIntentPermit,
    /// The order ID
    pub order_id: Uuid,
    /// The matching pool to which the intent is allocated
    pub matching_pool: String,
    /// Whether the intent allows external matches
    pub allow_external_matches: bool,
    /// The minimum fill size allowed for the intent
    pub min_fill_size: Amount,
}
