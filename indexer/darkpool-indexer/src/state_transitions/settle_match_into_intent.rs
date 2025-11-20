//! Defines the application-specific logic for settling a match into an intent
//! object

use renegade_constants::Scalar;

use crate::state_transitions::{StateApplicator, error::StateTransitionError};

// ---------
// | Types |
// ---------

/// A transition representing the settlement of a match into an intent object
#[derive(Clone)]
pub struct SettleMatchIntoIntentTransition {
    /// The now-spent nullifier of the intent being settled into
    pub nullifier: Scalar,
    /// The block number in which the match was settled
    pub block_number: u64,
    /// The public share of the new amount in the intent
    pub new_amount_public_share: Scalar,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Settle a match into an intent object
    pub async fn settle_match_into_intent(
        &self,
        transition: SettleMatchIntoIntentTransition,
    ) -> Result<(), StateTransitionError> {
        todo!()
    }
}
