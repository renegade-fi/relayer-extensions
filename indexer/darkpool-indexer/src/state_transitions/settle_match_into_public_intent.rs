//! Defines the application-specific logic for settling a match into a public
//! intent

use alloy::primitives::{B256, TxHash};
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::{Amount, fixed_point::FixedPoint};
use tracing::warn;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::PublicIntentStateObject,
};

// ---------
// | Types |
// ---------

/// A transition representing the settlement of a match into a public intent
#[derive(Clone)]
pub struct SettleMatchIntoPublicIntentTransition {
    /// The intent hash
    pub intent_hash: B256,
    /// The transaction hash in which the match settled
    pub tx_hash: TxHash,
    /// The block number in which the match settled
    pub block_number: u64,
    /// The data required to settle a match into a public intent
    pub public_intent_settlement_data: PublicIntentSettlementData,
}

/// The data required to settle a match into a public intent
#[derive(Clone)]
pub enum PublicIntentSettlementData {
    /// From an internal match (settleMatch)
    InternalMatch {
        /// The input amount on the obligation bundle
        amount_in: Amount,
    },
    /// From an external match (settleExternalMatch)
    ExternalMatch {
        /// The price of the match
        price: FixedPoint,
        /// The external party's input amount
        external_party_amount_in: Amount,
    },
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Settle a match into a public intent
    pub async fn settle_match_into_public_intent(
        &self,
        transition: SettleMatchIntoPublicIntentTransition,
    ) -> Result<(), StateTransitionError> {
        let SettleMatchIntoPublicIntentTransition {
            intent_hash,
            tx_hash,
            block_number,
            public_intent_settlement_data,
        } = transition;

        let mut conn = self.db_client.get_db_conn().await?;
        let mut public_intent =
            self.db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        apply_settlement_into_public_intent(&public_intent_settlement_data, &mut public_intent);

        conn.transaction(move |conn| {
            async move {
                // Check if the public intent update has already been processed, no-oping if so
                let public_intent_update_processed = self
                    .db_client
                    .check_public_intent_update_processed(intent_hash, tx_hash, conn)
                    .await?;

                if public_intent_update_processed {
                    warn!(
                        "Public intent update for intent hash {intent_hash} in tx {tx_hash} has \
                         already been processed, skipping update"
                    );

                    return Ok(());
                }

                // Update the public intent record
                self.db_client.update_public_intent(public_intent, conn).await?;

                // Mark the public intent update as processed
                self.db_client
                    .mark_public_intent_update_processed(intent_hash, tx_hash, block_number, conn)
                    .await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Apply the settlement based on settlement data variant
fn apply_settlement_into_public_intent(
    public_intent_settlement_data: &PublicIntentSettlementData,
    public_intent: &mut PublicIntentStateObject,
) {
    match public_intent_settlement_data {
        PublicIntentSettlementData::InternalMatch { amount_in } => {
            public_intent.order.intent.inner.amount_in -= amount_in;
        },
        PublicIntentSettlementData::ExternalMatch { price, external_party_amount_in } => {
            public_intent.update_from_external_match(*price, *external_party_amount_in);
        },
    };
}

#[cfg(test)]
mod tests {
    use super::PublicIntentSettlementData;
    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::{
            create_public_intent::PublicIntentCreationData,
            error::StateTransitionError,
            test_utils::{
                gen_create_public_intent_transition,
                gen_settle_external_match_into_public_intent_transition,
                gen_settle_match_into_public_intent_transition, register_random_master_view_seed,
                setup_test_state_applicator, validate_public_intent_indexing,
            },
        },
    };
    use renegade_crypto::fields::scalar_to_u128;

    /// Test that an internal match settlement into a public intent is indexed
    /// correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_internal_match_into_public_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let create_public_intent_transition =
            gen_create_public_intent_transition(master_view_seed.owner_address);

        let intent_hash = create_public_intent_transition.intent_hash;

        // Extract intent and creation amount_in, expecting InternalMatch
        let PublicIntentCreationData::InternalMatch {
            intent: initial_intent,
            amount_in: creation_amount_in,
        } = create_public_intent_transition.public_intent_creation_data.clone()
        else {
            panic!("Expected InternalMatch variant for creation");
        };

        // Index the initial public intent creation
        test_applicator.create_public_intent(create_public_intent_transition).await?;

        // Generate the subsequent internal match settlement transition
        let mut conn = db_client.get_db_conn().await?;
        let initial_public_intent =
            db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        let settle_match_transition =
            gen_settle_match_into_public_intent_transition(&initial_public_intent);

        let tx_hash = settle_match_transition.tx_hash;

        // Extract settlement amount_in, expecting InternalMatch
        let PublicIntentSettlementData::InternalMatch { amount_in: settlement_amount_in } =
            &settle_match_transition.public_intent_settlement_data
        else {
            panic!("Expected InternalMatch variant for settlement");
        };

        let mut expected_intent = initial_intent.clone();
        expected_intent.amount_in -= creation_amount_in;
        expected_intent.amount_in -= settlement_amount_in;

        // Index the match settlement
        test_applicator.settle_match_into_public_intent(settle_match_transition).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent update was marked as processed
        assert!(
            db_client.check_public_intent_update_processed(intent_hash, tx_hash, &mut conn).await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that an external match settlement into a public intent is indexed
    /// correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_external_match_into_public_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let create_public_intent_transition =
            gen_create_public_intent_transition(master_view_seed.owner_address);

        let intent_hash = create_public_intent_transition.intent_hash;

        // Extract intent and creation amount_in, expecting InternalMatch
        let PublicIntentCreationData::InternalMatch {
            intent: initial_intent,
            amount_in: creation_amount_in,
        } = create_public_intent_transition.public_intent_creation_data.clone()
        else {
            panic!("Expected InternalMatch variant for creation");
        };

        // Index the initial public intent creation
        test_applicator.create_public_intent(create_public_intent_transition).await?;

        // Generate the subsequent external match settlement transition
        let mut conn = db_client.get_db_conn().await?;
        let initial_public_intent =
            db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        let settle_match_transition =
            gen_settle_external_match_into_public_intent_transition(&initial_public_intent);

        let tx_hash = settle_match_transition.tx_hash;

        // Extract settlement data, expecting ExternalMatch
        let PublicIntentSettlementData::ExternalMatch { price, external_party_amount_in } =
            &settle_match_transition.public_intent_settlement_data
        else {
            panic!("Expected ExternalMatch variant for settlement");
        };

        // Compute the internal party amount_in from the external party amount and price
        let settlement_amount_in = scalar_to_u128(
            &price.inverse().expect("price is zero").floor_mul_int(*external_party_amount_in),
        );

        let mut expected_intent = initial_intent.clone();
        expected_intent.amount_in -= creation_amount_in;
        expected_intent.amount_in -= settlement_amount_in;

        // Index the match settlement
        test_applicator.settle_match_into_public_intent(settle_match_transition).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent update was marked as processed
        assert!(
            db_client.check_public_intent_update_processed(intent_hash, tx_hash, &mut conn).await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
