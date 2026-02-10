//! Defines the application-specific logic for settling a match into a public
//! intent, with upsert semantics (creates if not exists, updates if exists)

use alloy::primitives::{Address, B256, TxHash};
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::{Amount, fixed_point::FixedPoint};
use renegade_darkpool_types::intent::Intent;
use renegade_solidity_abi::v2::IDarkpoolV2::{PublicIntentPermit, SignatureWithNonce};
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::PublicIntentStateObject,
};

// ---------
// | Types |
// ---------

/// A transition representing the settlement of a match into a public intent
#[derive(Clone)]
pub struct SettlePublicIntentTransition {
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
        /// The public intent
        intent: Intent,
        /// The intent signature
        intent_signature: SignatureWithNonce,
        /// The permit for the intent
        permit: PublicIntentPermit,
        /// The input amount on the obligation bundle
        amount_in: Amount,
    },
    /// From an external match (settleExternalMatch)
    ExternalMatch {
        /// The public intent
        intent: Intent,
        /// The intent signature
        intent_signature: SignatureWithNonce,
        /// The permit for the intent
        permit: PublicIntentPermit,
        /// The price of the match
        price: FixedPoint,
        /// The external party's input amount
        external_party_amount_in: Amount,
    },
}

impl PublicIntentSettlementData {
    /// Get the owner address from the settlement data
    pub fn get_owner(&self) -> Address {
        match self {
            PublicIntentSettlementData::InternalMatch { intent, .. } => intent.owner,
            PublicIntentSettlementData::ExternalMatch { intent, .. } => intent.owner,
        }
    }
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Settle a match into a public intent (creates if not exists, updates if
    /// exists)
    #[instrument(skip_all, fields(intent_hash = %transition.intent_hash))]
    pub async fn settle_public_intent(
        &self,
        transition: SettlePublicIntentTransition,
        is_backfill: bool,
    ) -> Result<(), StateTransitionError> {
        let SettlePublicIntentTransition {
            intent_hash,
            tx_hash,
            block_number,
            public_intent_settlement_data,
        } = transition;

        let mut conn = self.db_client.get_db_conn().await?;

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
                         already been processed, skipping"
                    );

                    return Ok(());
                }

                // Upsert: check if public intent exists and branch accordingly
                let public_intent_exists =
                    self.db_client.public_intent_exists(intent_hash, conn).await?;

                if public_intent_exists {
                    // Update existing: decrement amount
                    let mut public_intent =
                        self.db_client.get_public_intent_by_hash(intent_hash, conn).await?;

                    apply_settlement(&public_intent_settlement_data, &mut public_intent);
                    self.db_client.update_public_intent(public_intent, conn).await?;
                } else {
                    // Create new: construct from settlement data
                    let master_view_seed = self
                        .db_client
                        .get_master_view_seed_by_owner_address(
                            public_intent_settlement_data.get_owner(),
                            conn,
                        )
                        .await?;

                    let public_intent = construct_new_public_intent(
                        public_intent_settlement_data,
                        intent_hash,
                        master_view_seed.account_id,
                    );

                    self.db_client.create_public_intent(public_intent, conn).await?;
                }

                // Mark the public intent update as processed
                self.db_client
                    .mark_public_intent_update_processed(
                        intent_hash,
                        tx_hash,
                        block_number,
                        is_backfill,
                        conn,
                    )
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
fn apply_settlement(
    public_intent_settlement_data: &PublicIntentSettlementData,
    public_intent: &mut PublicIntentStateObject,
) {
    match public_intent_settlement_data {
        PublicIntentSettlementData::InternalMatch { amount_in, .. } => {
            public_intent.order.decrement_amount_in(*amount_in);
        },
        PublicIntentSettlementData::ExternalMatch { price, external_party_amount_in, .. } => {
            public_intent.update_from_external_match(*price, *external_party_amount_in);
        },
    };
}

/// Construct a new public intent state object from the given settlement data
fn construct_new_public_intent(
    public_intent_settlement_data: PublicIntentSettlementData,
    intent_hash: B256,
    account_id: Uuid,
) -> PublicIntentStateObject {
    match public_intent_settlement_data {
        PublicIntentSettlementData::InternalMatch {
            intent,
            intent_signature,
            permit,
            amount_in,
        } => {
            let mut public_intent = PublicIntentStateObject::new(
                intent_hash,
                intent,
                intent_signature,
                permit,
                account_id,
            );
            public_intent.order.decrement_amount_in(amount_in);
            public_intent
        },
        PublicIntentSettlementData::ExternalMatch {
            intent,
            intent_signature,
            permit,
            price,
            external_party_amount_in,
        } => {
            let mut public_intent = PublicIntentStateObject::new(
                intent_hash,
                intent,
                intent_signature,
                permit,
                account_id,
            );

            public_intent.update_from_external_match(price, external_party_amount_in);
            public_intent
        },
    }
}

#[cfg(test)]
mod tests {
    use super::PublicIntentSettlementData;
    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::{
            error::StateTransitionError,
            test_utils::{
                gen_settle_public_intent_external_match_transition,
                gen_settle_public_intent_external_match_transition_for_existing,
                gen_settle_public_intent_transition,
                gen_settle_public_intent_transition_for_existing, register_random_master_view_seed,
                setup_test_state_applicator, validate_public_intent_indexing,
            },
        },
    };
    use renegade_crypto::fields::scalar_to_u128;

    /// Test that a public intent settlement creates a new record when the
    /// intent does not exist (internal match)
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_public_intent_creates_internal_match() -> Result<(), StateTransitionError>
    {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let transition = gen_settle_public_intent_transition(master_view_seed.owner_address);

        let intent_hash = transition.intent_hash;
        let tx_hash = transition.tx_hash;

        // Extract intent and amount_in from the settlement data
        let PublicIntentSettlementData::InternalMatch { intent, amount_in, .. } =
            &transition.public_intent_settlement_data
        else {
            panic!("Expected InternalMatch variant");
        };

        let mut expected_intent = intent.clone();
        expected_intent.amount_in -= amount_in;

        // Settle the public intent (should create since it doesn't exist)
        test_applicator.settle_public_intent(transition, false /* is_backfill */).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent update was marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(
            db_client.check_public_intent_update_processed(intent_hash, tx_hash, &mut conn).await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a public intent settlement creates a new record when the
    /// intent does not exist (external match)
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_public_intent_creates_external_match() -> Result<(), StateTransitionError>
    {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let transition =
            gen_settle_public_intent_external_match_transition(master_view_seed.owner_address);

        let intent_hash = transition.intent_hash;
        let tx_hash = transition.tx_hash;

        // Extract intent and external match data
        let PublicIntentSettlementData::ExternalMatch {
            intent,
            price,
            external_party_amount_in,
            ..
        } = &transition.public_intent_settlement_data
        else {
            panic!("Expected ExternalMatch variant");
        };

        // Compute the internal party amount_in from the external party amount and price
        let internal_amount_in = scalar_to_u128(
            &price.inverse().expect("price is zero").floor_mul_int(*external_party_amount_in),
        );

        let mut expected_intent = intent.clone();
        expected_intent.amount_in -= internal_amount_in;

        // Settle the public intent (should create since it doesn't exist)
        test_applicator.settle_public_intent(transition, false /* is_backfill */).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent update was marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(
            db_client.check_public_intent_update_processed(intent_hash, tx_hash, &mut conn).await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a public intent settlement updates an existing record
    /// (internal match)
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_public_intent_updates_internal_match() -> Result<(), StateTransitionError>
    {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;

        // First, create a public intent
        let create_transition = gen_settle_public_intent_transition(master_view_seed.owner_address);
        let intent_hash = create_transition.intent_hash;

        let PublicIntentSettlementData::InternalMatch {
            intent: initial_intent,
            amount_in: creation_amount_in,
            ..
        } = &create_transition.public_intent_settlement_data
        else {
            panic!("Expected InternalMatch variant for creation");
        };

        let initial_intent = initial_intent.clone();
        let creation_amount_in = *creation_amount_in;

        test_applicator.settle_public_intent(create_transition, false /* is_backfill */).await?;

        // Now generate a subsequent settlement transition
        let mut conn = db_client.get_db_conn().await?;
        let initial_public_intent =
            db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        let update_transition =
            gen_settle_public_intent_transition_for_existing(&initial_public_intent);
        let tx_hash = update_transition.tx_hash;

        let PublicIntentSettlementData::InternalMatch { amount_in: settlement_amount_in, .. } =
            &update_transition.public_intent_settlement_data
        else {
            panic!("Expected InternalMatch variant for settlement");
        };

        let mut expected_intent = initial_intent;
        expected_intent.amount_in -= creation_amount_in;
        expected_intent.amount_in -= settlement_amount_in;

        // Settle the public intent (should update since it exists)
        test_applicator.settle_public_intent(update_transition, false /* is_backfill */).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent update was marked as processed
        assert!(
            db_client.check_public_intent_update_processed(intent_hash, tx_hash, &mut conn).await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a public intent settlement updates an existing record
    /// (external match)
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_public_intent_updates_external_match() -> Result<(), StateTransitionError>
    {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;

        // First, create a public intent
        let create_transition = gen_settle_public_intent_transition(master_view_seed.owner_address);
        let intent_hash = create_transition.intent_hash;

        let PublicIntentSettlementData::InternalMatch {
            intent: initial_intent,
            amount_in: creation_amount_in,
            ..
        } = &create_transition.public_intent_settlement_data
        else {
            panic!("Expected InternalMatch variant for creation");
        };

        let initial_intent = initial_intent.clone();
        let creation_amount_in = *creation_amount_in;

        test_applicator.settle_public_intent(create_transition, false /* is_backfill */).await?;

        // Now generate a subsequent external match settlement transition
        let mut conn = db_client.get_db_conn().await?;
        let initial_public_intent =
            db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        let update_transition =
            gen_settle_public_intent_external_match_transition_for_existing(&initial_public_intent);
        let tx_hash = update_transition.tx_hash;

        let PublicIntentSettlementData::ExternalMatch { price, external_party_amount_in, .. } =
            &update_transition.public_intent_settlement_data
        else {
            panic!("Expected ExternalMatch variant for settlement");
        };

        // Compute the internal party amount_in from the external party amount and price
        let settlement_amount_in = scalar_to_u128(
            &price.inverse().expect("price is zero").floor_mul_int(*external_party_amount_in),
        );

        let mut expected_intent = initial_intent;
        expected_intent.amount_in -= creation_amount_in;
        expected_intent.amount_in -= settlement_amount_in;

        // Settle the public intent (should update since it exists)
        test_applicator.settle_public_intent(update_transition, false /* is_backfill */).await?;

        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Assert that the public intent update was marked as processed
        assert!(
            db_client.check_public_intent_update_processed(intent_hash, tx_hash, &mut conn).await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
