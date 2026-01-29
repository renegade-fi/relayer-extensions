//! Defines the application-specific logic for creating a new public intent

use alloy::primitives::{Address, B256, TxHash};
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::{Amount, fixed_point::FixedPoint};
use renegade_darkpool_types::intent::Intent;
use tracing::warn;
use uuid::Uuid;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::PublicIntentStateObject,
};

// ---------
// | Types |
// ---------

/// A transition representing the creation of a new public intent
#[derive(Clone)]
pub struct CreatePublicIntentTransition {
    /// The intent hash
    pub intent_hash: B256,
    /// The transaction hash in which the public intent was created
    pub tx_hash: TxHash,
    /// The block number in which the public intent was created
    pub block_number: u64,
    /// The data required to create a new public intent
    pub public_intent_creation_data: PublicIntentCreationData,
}

/// The data required to create a new public intent
#[derive(Clone)]
pub enum PublicIntentCreationData {
    /// From an internal match (settleMatch)
    InternalMatch {
        /// The public intent to create
        intent: Intent,
        /// The input amount on the obligation bundle
        amount_in: Amount,
    },
    /// From an external match (settleExternalMatch)
    ExternalMatch {
        /// The public intent to create
        intent: Intent,
        /// The price of the match
        price: FixedPoint,
        /// The external party's input amount
        external_party_amount_in: Amount,
    },
}

impl PublicIntentCreationData {
    /// Get the owner address from the creation data
    pub fn get_owner(&self) -> Address {
        match self {
            PublicIntentCreationData::InternalMatch { intent, .. } => intent.owner,
            PublicIntentCreationData::ExternalMatch { intent, .. } => intent.owner,
        }
    }
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Create a new public intent
    pub async fn create_public_intent(
        &self,
        transition: CreatePublicIntentTransition,
    ) -> Result<(), StateTransitionError> {
        let CreatePublicIntentTransition {
            intent_hash,
            tx_hash,
            block_number,
            public_intent_creation_data,
        } = transition;

        let mut conn = self.db_client.get_db_conn().await?;

        let master_view_seed = self
            .db_client
            .get_master_view_seed_by_owner_address(
                public_intent_creation_data.get_owner(),
                &mut conn,
            )
            .await?;

        let public_intent = construct_new_public_intent(
            public_intent_creation_data,
            intent_hash,
            master_view_seed.account_id,
        );

        conn.transaction(move |conn| {
            async move {
                // Check if the public intent creation has already been processed, no-oping if so
                let public_intent_creation_processed = self.db_client.check_public_intent_creation_processed(intent_hash, tx_hash, conn).await?;

                if public_intent_creation_processed {
                    warn!(
                        "Public intent creation for intent hash {intent_hash} has already been processed, skipping creation"
                    );

                    return Ok(());
                }

                // Mark the public intent creation as processed
                self.db_client.mark_public_intent_creation_processed(intent_hash, tx_hash, block_number, conn).await?;

                // Check if a public intent record already exists for the intent hash.
                // This is possible in the case that we previously processed a metadata update message for the public intent.
                let public_intent_exists = self.db_client.public_intent_exists(intent_hash, conn).await?;
                if public_intent_exists {
                    warn!(
                        "Public intent record already exists for intent hash {intent_hash}, skipping creation"
                    );

                    return Ok(());
                }

                // Insert the new public intent record
                self.db_client.create_public_intent(public_intent, conn).await?;

                Ok(())
            }.scope_boxed()
        }).await
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Construct a new public intent state object from the given creation data
fn construct_new_public_intent(
    public_intent_creation_data: PublicIntentCreationData,
    intent_hash: B256,
    account_id: Uuid,
) -> PublicIntentStateObject {
    match public_intent_creation_data {
        PublicIntentCreationData::InternalMatch { amount_in, mut intent } => {
            intent.amount_in -= amount_in;
            PublicIntentStateObject::new(intent_hash, intent, account_id)
        },
        PublicIntentCreationData::ExternalMatch { price, external_party_amount_in, intent } => {
            let mut public_intent = PublicIntentStateObject::new(intent_hash, intent, account_id);
            public_intent.update_from_external_match(price, external_party_amount_in);
            public_intent
        },
    }
}

#[cfg(test)]
mod tests {
    use super::PublicIntentCreationData;
    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::{
            error::StateTransitionError,
            test_utils::{
                gen_create_public_intent_external_match_transition,
                gen_create_public_intent_transition, register_random_master_view_seed,
                setup_test_state_applicator, validate_public_intent_indexing,
            },
        },
    };
    use renegade_crypto::fields::scalar_to_u128;

    /// Test that a public intent creation from an internal match is indexed
    /// correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_public_intent_internal_match() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let transition = gen_create_public_intent_transition(master_view_seed.owner_address);

        let intent_hash = transition.intent_hash;
        let tx_hash = transition.tx_hash;

        // Extract intent and amount_in from the creation data, expecting InternalMatch
        let PublicIntentCreationData::InternalMatch { intent, amount_in } =
            &transition.public_intent_creation_data
        else {
            panic!("Expected InternalMatch variant");
        };

        let mut expected_intent = intent.clone();
        expected_intent.amount_in -= amount_in;

        // Index the public intent creation
        test_applicator.create_public_intent(transition).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent creation was marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(
            db_client
                .check_public_intent_creation_processed(intent_hash, tx_hash, &mut conn)
                .await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a public intent creation from an external match is indexed
    /// correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_public_intent_external_match() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let transition =
            gen_create_public_intent_external_match_transition(master_view_seed.owner_address);

        let intent_hash = transition.intent_hash;
        let tx_hash = transition.tx_hash;

        // Extract intent and external match data, expecting ExternalMatch
        let PublicIntentCreationData::ExternalMatch { intent, price, external_party_amount_in } =
            &transition.public_intent_creation_data
        else {
            panic!("Expected ExternalMatch variant");
        };

        // Compute the internal party amount_in from the external party amount and price
        let internal_amount_in = scalar_to_u128(
            &price.inverse().expect("price is zero").floor_mul_int(*external_party_amount_in),
        );

        let mut expected_intent = intent.clone();
        expected_intent.amount_in -= internal_amount_in;

        // Index the public intent creation
        test_applicator.create_public_intent(transition).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent creation was marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(
            db_client
                .check_public_intent_creation_processed(intent_hash, tx_hash, &mut conn)
                .await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
