//! Defines the process for backfilling a user's state

use alloy::primitives::TxHash;
use darkpool_indexer_api::types::message_queue::{
    CancelPublicIntentMessage, Message, NullifierSpendMessage, RecoveryIdMessage,
    UpdatePublicIntentMessage,
};
use renegade_constants::Scalar;
use renegade_darkpool_types::csprng::PoseidonCSPRNG;
use tokio::task::JoinSet;
use tracing::{error, info, instrument};
use uuid::Uuid;

use crate::{
    crypto_mocks::recovery_stream::sample_next_nullifier,
    darkpool_client::utils::scalar_to_b256,
    indexer::{Indexer, error::IndexerError},
    message_queue::MessageQueue,
    types::MasterViewSeed,
};

impl Indexer {
    /// Backfill a user's state
    #[instrument(skip(self))]
    pub async fn backfill_user_state(&self, account_id: Uuid) -> Result<(), IndexerError> {
        info!("Backfilling state for account {account_id}");

        let mut conn = self.db_client.get_db_conn().await?;
        let master_view_seed =
            self.db_client.get_master_view_seed_by_account_id(account_id, &mut conn).await?;

        // Run both backfills in parallel
        let (state_result, public_result) = tokio::join!(
            self.backfill_state_objects(&master_view_seed),
            self.backfill_public_intents(&master_view_seed)
        );

        if let Err(e) = state_result {
            error!("Error backfilling state objects for account {account_id}: {e}");
        }
        if let Err(e) = public_result {
            error!("Error backfilling public intents for account {account_id}: {e}");
        }

        Ok(())
    }

    /// Backfill state objects (balances & intents) for a user
    async fn backfill_state_objects(
        &self,
        master_view_seed: &MasterViewSeed,
    ) -> Result<(), IndexerError> {
        // We restart our view of the master view seed's recovery seed CSPRNG so that we
        // can backfill from the very beginning of user state history
        let mut recovery_seed_csprng = master_view_seed.recovery_seed_csprng.clone();
        recovery_seed_csprng.index = 0;

        let mut object_backfill_tasks = JoinSet::new();

        loop {
            let recovery_stream_seed = recovery_seed_csprng.next().unwrap();
            let recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);

            info!("Backfilling state for object with recovery stream seed {recovery_stream_seed}");

            // If the object is already indexed, backfill its state updates starting
            // from the last-indexed nullifier
            let maybe_nullifier =
                self.try_get_existing_object_nullifier(recovery_stream_seed).await?;

            if let Some(nullifier) = maybe_nullifier {
                let self_clone = self.clone();
                object_backfill_tasks.spawn(async move {
                    let result =
                        self_clone.backfill_object_from_nullifier(nullifier, recovery_stream).await;

                    (recovery_stream_seed, result)
                });

                continue;
            }

            // Otherwise, check if the first recovery ID has been registered
            let first_recovery_id = recovery_stream.get_ith(0);
            let maybe_registration_tx = self.try_get_registration_tx(first_recovery_id).await?;

            // If this object has never been registered, then any subsequent objects for the
            // given account haven't, either.
            // As such, we break out of the loop
            // as there are no more per-object backfill tasks to spawn.
            if maybe_registration_tx.is_none() {
                break;
            }

            // Otherwise, we backfill its state updates starting from the registration event
            let tx_hash = maybe_registration_tx.unwrap();

            let self_clone = self.clone();
            object_backfill_tasks.spawn(async move {
                let result = self_clone
                    .backfill_object_from_registration(first_recovery_id, tx_hash, recovery_stream)
                    .await;

                (recovery_stream_seed, result)
            });
        }

        let mut final_result = Ok(());
        let results = object_backfill_tasks.join_all().await;
        for (recovery_stream_seed, result) in results {
            if let Err(e) = result {
                final_result = Err(IndexerError::Backfill(master_view_seed.account_id));
                error!(
                    "Error backfilling state for object with recovery stream seed {recovery_stream_seed}: {e}"
                );
            }
        }

        final_result
    }

    /// Backfill public intents (updates + cancellations) for a user
    async fn backfill_public_intents(
        &self,
        master_view_seed: &MasterViewSeed,
    ) -> Result<(), IndexerError> {
        let owner_address = master_view_seed.owner_address;
        let owner_topic = owner_address.into_word();

        // Query all PublicIntentUpdated events for this owner
        let update_filter =
            self.darkpool_client.darkpool.PublicIntentUpdated_filter().topic2(owner_topic);

        let update_events = update_filter.query().await.map_err(IndexerError::rpc)?;

        info!(
            "Found {} public intent update events for owner {}",
            update_events.len(),
            owner_address
        );

        // Enqueue update messages in order
        for (event, log) in &update_events {
            let intent_hash = event.intentHash;
            let tx_hash = log.transaction_hash.ok_or(IndexerError::rpc(format!(
                "no tx hash for public intent {intent_hash} update event during backfill"
            )))?;

            let message = Message::UpdatePublicIntent(UpdatePublicIntentMessage {
                intent_hash,
                tx_hash,
                is_backfill: true,
            });

            let intent_hash_str = intent_hash.to_string();
            let tx_hash_str = tx_hash.to_string();

            self.message_queue.send_message(message, tx_hash_str, intent_hash_str).await?;
        }

        // Query all PublicIntentCancelled events for this owner
        let cancel_filter =
            self.darkpool_client.darkpool.PublicIntentCancelled_filter().topic2(owner_topic);

        let cancel_events = cancel_filter.query().await.map_err(IndexerError::rpc)?;

        info!(
            "Found {} public intent cancellation events for owner {}",
            cancel_events.len(),
            owner_address
        );

        // Enqueue cancellation messages (using same message group as updates so they
        // process after updates for the same intent_hash)
        for (event, log) in &cancel_events {
            let intent_hash = event.intentHash;
            let tx_hash = log.transaction_hash.ok_or(IndexerError::rpc(format!(
                "no tx hash for public intent {intent_hash} cancellation event during backfill"
            )))?;

            let message = Message::CancelPublicIntent(CancelPublicIntentMessage {
                intent_hash,
                tx_hash,
                is_backfill: true,
            });

            let intent_hash_str = intent_hash.to_string();
            let tx_hash_str = tx_hash.to_string();

            self.message_queue.send_message(message, tx_hash_str, intent_hash_str).await?;
        }

        Ok(())
    }

    /// Get the current nullifier of the state object associated with the given
    /// recovery stream seed, if any
    async fn try_get_existing_object_nullifier(
        &self,
        recovery_stream_seed: Scalar,
    ) -> Result<Option<Scalar>, IndexerError> {
        let mut conn = self.db_client.get_db_conn().await?;
        let maybe_balance_nullifier = self
            .db_client
            .get_balance_by_recovery_stream_seed(recovery_stream_seed, &mut conn)
            .await?
            .map(|balance| balance.balance.compute_nullifier());

        let maybe_intent_nullifier = self
            .db_client
            .get_intent_by_recovery_stream_seed(recovery_stream_seed, &mut conn)
            .await?
            .map(|intent| intent.intent.compute_nullifier());

        Ok(maybe_balance_nullifier.xor(maybe_intent_nullifier))
    }

    /// Try to get the transaction hash of the registration of the given
    /// recovery ID, if it has been registered
    async fn try_get_registration_tx(
        &self,
        recovery_id: Scalar,
    ) -> Result<Option<TxHash>, IndexerError> {
        let recovery_id_topic = scalar_to_b256(recovery_id);
        let registration_event_filter =
            self.darkpool_client.darkpool.RecoveryIdRegistered_filter().topic1(recovery_id_topic);

        let maybe_registration_event =
            registration_event_filter.query_raw().await?.first().cloned();

        let maybe_tx_hash = maybe_registration_event.and_then(|event| event.transaction_hash);
        Ok(maybe_tx_hash)
    }

    /// Backfill an object's historic state updates, starting from the spend of
    /// the given nullifier
    async fn backfill_object_from_nullifier(
        &self,
        mut nullifier: Scalar,
        mut recovery_stream: PoseidonCSPRNG,
    ) -> Result<(), IndexerError> {
        info!(
            "Backfilling state for object with recovery stream seed {} starting from nullifier {nullifier}",
            recovery_stream.seed
        );

        loop {
            let nullifier_topic = scalar_to_b256(nullifier);
            let nullifier_spend_filter =
                self.darkpool_client.darkpool.NullifierSpent_filter().topic1(nullifier_topic);

            let maybe_spend_event = nullifier_spend_filter.query_raw().await?.first().cloned();

            // If there is no spend event for the nullifier, the backfill is complete
            if maybe_spend_event.is_none() {
                break Ok(());
            }

            // Otherwise, we enqueue a nullifier spend message for subsequent indexing
            let spend_event = maybe_spend_event.unwrap();
            let tx_hash = spend_event.transaction_hash.ok_or(IndexerError::rpc(format!(
                "no tx hash for nullifier {nullifier} spend event"
            )))?;

            let nullifier_spend_message = Message::NullifierSpend(NullifierSpendMessage {
                nullifier,
                tx_hash,
                is_backfill: true,
            });

            // We use the object's recovery stream seed as a message group ID so that all
            // messages enqueued by this backfill task are processed sequentially
            self.message_queue
                .send_message(
                    nullifier_spend_message,
                    nullifier.to_string(),
                    recovery_stream.seed.to_string(),
                )
                .await?;

            // Finally, we compute the next nullifier & repeat
            nullifier = sample_next_nullifier(&mut recovery_stream);
        }
    }

    /// Backfill an object's historic state updates, starting from the
    /// registration of the the given recovery ID (assumed to be the first)
    async fn backfill_object_from_registration(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
        mut recovery_stream: PoseidonCSPRNG,
    ) -> Result<(), IndexerError> {
        info!(
            "Backfilling state for object with recovery stream seed {} starting from registration of recovery ID {recovery_id}",
            recovery_stream.seed
        );

        let recovery_id_message = Message::RegisterRecoveryId(RecoveryIdMessage {
            recovery_id,
            tx_hash,
            is_backfill: true,
        });

        // We use the object's recovery stream seed as a message group ID so that all
        // messages enqueued by this backfill task are processed sequentially
        self.message_queue
            .send_message(
                recovery_id_message,
                recovery_id.to_string(),
                recovery_stream.seed.to_string(),
            )
            .await?;

        // We then sample the first nullifier for the object, and proceed to the main
        // backfill process
        let first_nullifier = sample_next_nullifier(&mut recovery_stream);
        self.backfill_object_from_nullifier(first_nullifier, recovery_stream).await
    }
}
