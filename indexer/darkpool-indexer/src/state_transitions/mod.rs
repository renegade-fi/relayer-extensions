//! Defines the application-specific logic for applying transitions to state
//! objects.
//!
//! In the interest of logical separation & ease of testing, these transitions
//! are defined over canonical object types & have no dependencies on external
//! resources like the database or RPC client.

use darkpool_indexer_api::types::message_queue::{
    CancelPublicIntentMessage, MasterViewSeedMessage, PublicIntentMetadataUpdateMessage,
};

use crate::{
    db::client::DbClient,
    state_transitions::{
        cancel_order::CancelOrderTransition, create_balance::CreateBalanceTransition,
        create_intent::CreateIntentTransition, deposit::DepositTransition,
        error::StateTransitionError, pay_protocol_fee::PayProtocolFeeTransition,
        pay_relayer_fee::PayRelayerFeeTransition,
        settle_match_into_balance::SettleMatchIntoBalanceTransition,
        settle_match_into_intent::SettleMatchIntoIntentTransition,
        settle_public_intent::SettlePublicIntentTransition, withdraw::WithdrawTransition,
    },
};

pub mod cancel_order;
pub mod cancel_public_intent;
pub mod create_balance;
pub mod create_intent;
pub mod deposit;
pub mod error;
pub mod pay_protocol_fee;
pub mod pay_relayer_fee;
pub mod register_master_view_seed;
pub mod settle_match_into_balance;
pub mod settle_match_into_intent;
pub mod settle_public_intent;
pub mod update_public_intent_metadata;
pub mod withdraw;

#[cfg(test)]
mod test_utils;

// ---------
// | Types |
// ---------

/// The type of a state transition
#[allow(clippy::large_enum_variant)]
pub enum StateTransition {
    /// The registration of a new master view seed
    RegisterMasterViewSeed(MasterViewSeedMessage),
    /// The creation of a new balance object
    CreateBalance(CreateBalanceTransition),
    /// The deposit of funds into an existing balance object
    Deposit(DepositTransition),
    /// The withdrawal of funds from an existing balance object
    Withdraw(WithdrawTransition),
    /// The payment of the protocol fee accrued on a balance object
    PayProtocolFee(PayProtocolFeeTransition),
    /// The payment of the relayer fee accrued on a balance object
    PayRelayerFee(PayRelayerFeeTransition),
    /// The settlement of a match into a balance object
    SettleMatchIntoBalance(SettleMatchIntoBalanceTransition),
    /// The creation of a new intent object
    CreateIntent(CreateIntentTransition),
    /// The settlement of a match into an intent object
    SettleMatchIntoIntent(SettleMatchIntoIntentTransition),
    /// The settlement of a match into a public intent (upsert semantics)
    SettlePublicIntent(SettlePublicIntentTransition),
    /// An update to a public intent's metadata (upsert semantics)
    UpdatePublicIntentMetadata(PublicIntentMetadataUpdateMessage),
    /// The cancellation of an order
    CancelOrder(CancelOrderTransition),
    /// The cancellation of a public intent
    CancelPublicIntent(CancelPublicIntentMessage),
}

impl StateTransition {
    /// Get a human-readable name for the state transition
    pub fn name(&self) -> String {
        match self {
            StateTransition::RegisterMasterViewSeed(_) => "RegisterMasterViewSeed".to_string(),
            StateTransition::CreateBalance(_) => "CreateBalance".to_string(),
            StateTransition::Deposit(_) => "Deposit".to_string(),
            StateTransition::Withdraw(_) => "Withdraw".to_string(),
            StateTransition::PayProtocolFee(_) => "PayProtocolFee".to_string(),
            StateTransition::PayRelayerFee(_) => "PayRelayerFee".to_string(),
            StateTransition::SettleMatchIntoBalance(_) => "SettleMatchIntoBalance".to_string(),
            StateTransition::CreateIntent(_) => "CreateIntent".to_string(),
            StateTransition::SettleMatchIntoIntent(_) => "SettleMatchIntoIntent".to_string(),
            StateTransition::SettlePublicIntent(_) => "SettlePublicIntent".to_string(),
            StateTransition::UpdatePublicIntentMetadata(_) => {
                "UpdatePublicIntentMetadata".to_string()
            },
            StateTransition::CancelOrder(_) => "CancelOrder".to_string(),
            StateTransition::CancelPublicIntent(_) => "CancelPublicIntent".to_string(),
        }
    }
}

/// The state applicator, responsible for applying high-level state transitions
/// to the database
#[derive(Clone)]
pub struct StateApplicator {
    /// The DB client
    pub db_client: DbClient,
}

impl StateApplicator {
    /// Create a new state applicator
    pub fn new(db_client: DbClient) -> Self {
        Self { db_client }
    }

    /// Apply a state transition to the database
    pub async fn apply_state_transition(
        &self,
        transition: StateTransition,
        is_backfill: bool,
    ) -> Result<(), StateTransitionError> {
        match transition {
            StateTransition::CreateBalance(transition) => {
                self.create_balance(transition, is_backfill).await
            },
            StateTransition::RegisterMasterViewSeed(transition) => {
                self.register_master_view_seed(transition).await
            },
            StateTransition::Deposit(transition) => self.deposit(transition, is_backfill).await,
            StateTransition::Withdraw(transition) => self.withdraw(transition, is_backfill).await,
            StateTransition::PayProtocolFee(transition) => {
                self.pay_protocol_fee(transition, is_backfill).await
            },
            StateTransition::PayRelayerFee(transition) => {
                self.pay_relayer_fee(transition, is_backfill).await
            },
            StateTransition::SettleMatchIntoBalance(transition) => {
                self.settle_match_into_balance(transition, is_backfill).await
            },
            StateTransition::CreateIntent(transition) => {
                self.create_intent(transition, is_backfill).await
            },
            StateTransition::SettleMatchIntoIntent(transition) => {
                self.settle_match_into_intent(transition, is_backfill).await
            },
            StateTransition::SettlePublicIntent(transition) => {
                self.settle_public_intent(transition, is_backfill).await
            },
            StateTransition::UpdatePublicIntentMetadata(message) => {
                self.update_public_intent_metadata(message).await
            },
            StateTransition::CancelOrder(transition) => {
                self.cancel_order(transition, is_backfill).await
            },
            StateTransition::CancelPublicIntent(message) => {
                self.cancel_public_intent(message).await
            },
        }
    }
}
