//! Defines the application-specific logic for applying transitions to state
//! objects.
//!
//! In the interest of logical separation & ease of testing, these transitions
//! are defined over canonical object types & have no dependencies on external
//! resources like the database or RPC client.

use crate::{
    db::client::DbClient,
    state_transitions::{error::StateTransitionError, types::StateTransition},
};

pub mod create_balance;
pub mod error;
pub mod register_master_view_seed;
pub mod types;

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
        }
    }
}
