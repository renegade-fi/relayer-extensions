//! Defines the types of state transitions that can be applied to state objects.

use darkpool_indexer_api::types::sqs::MasterViewSeedMessage;
use renegade_circuit_types::balance::BalanceShare;
use renegade_constants::Scalar;

/// The type of a state transition
pub enum StateTransition {
    /// The registration of a new master view seed
    RegisterMasterViewSeed(MasterViewSeedMessage),
    /// The creation of a new balance object
    CreateBalance(CreateBalanceTransition),
}

/// A transition representing the creation of a new balance object
pub struct CreateBalanceTransition {
    /// The recovery ID registered for the balance
    pub recovery_id: Scalar,
    /// The block number in which the recovery ID was registered
    pub block_number: u64,
    /// The public shares of the balance
    pub public_share: BalanceShare,
}
