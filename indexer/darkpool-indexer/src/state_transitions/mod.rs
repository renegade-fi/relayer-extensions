//! Defines the application-specific logic for applying transitions to state
//! objects.
//!
//! In the interest of logical separation & ease of testing, these transitions
//! are defined over canonical object types & have no dependencies on external
//! resources like the database or RPC client.

use darkpool_indexer_api::types::sqs::MasterViewSeedMessage;

use crate::{
    db::client::DbClient,
    state_transitions::{
        create_balance::CreateBalanceTransition, create_intent::CreateIntentTransition,
        create_public_intent::CreatePublicIntentTransition, deposit::DepositTransition,
        error::StateTransitionError, pay_protocol_fee::PayProtocolFeeTransition,
        pay_relayer_fee::PayRelayerFeeTransition,
        settle_match_into_balance::SettleMatchIntoBalanceTransition,
        settle_match_into_public_intent::SettleMatchIntoPublicIntentTransition,
        withdraw::WithdrawTransition,
    },
};

pub mod create_balance;
pub mod create_intent;
pub mod create_public_intent;
pub mod deposit;
pub mod error;
pub mod pay_protocol_fee;
pub mod pay_relayer_fee;
pub mod register_master_view_seed;
pub mod settle_match_into_balance;
pub mod settle_match_into_public_intent;
pub mod withdraw;

#[cfg(test)]
mod test_utils;

// ---------
// | Types |
// ---------

/// The type of a state transition
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
    /// The creation of a new public intent
    CreatePublicIntent(CreatePublicIntentTransition),
    /// The settlement of a match into a public intent
    SettleMatchIntoPublicIntent(SettleMatchIntoPublicIntentTransition),
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
    ) -> Result<(), StateTransitionError> {
        match transition {
            StateTransition::CreateBalance(transition) => self.create_balance(transition).await,
            StateTransition::RegisterMasterViewSeed(transition) => {
                self.register_master_view_seed(transition).await
            },
            StateTransition::Deposit(transition) => self.deposit(transition).await,
            StateTransition::Withdraw(transition) => self.withdraw(transition).await,
            StateTransition::PayProtocolFee(transition) => self.pay_protocol_fee(transition).await,
            StateTransition::PayRelayerFee(transition) => self.pay_relayer_fee(transition).await,
            StateTransition::SettleMatchIntoBalance(transition) => {
                self.settle_match_into_balance(transition).await
            },
            StateTransition::CreateIntent(transition) => self.create_intent(transition).await,
            StateTransition::CreatePublicIntent(transition) => {
                self.create_public_intent(transition).await
            },
            StateTransition::SettleMatchIntoPublicIntent(transition) => {
                self.settle_match_into_public_intent(transition).await
            },
        }
    }
}
