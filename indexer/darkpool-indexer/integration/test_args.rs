//! Defines the arguments passed to every test

use std::{cell::OnceCell, path::PathBuf, str::FromStr, sync::Arc};

use alloy::{
    primitives::{Address, TxHash, U256},
    providers::{DynProvider, Provider, ext::AnvilApi},
    signers::local::PrivateKeySigner,
};
use darkpool_indexer::{
    chain_event_listener::ChainEventListener,
    darkpool_client::DarkpoolClient,
    db::{
        client::DbClient,
        test_utils::{cleanup_test_db, setup_test_db},
    },
    indexer::{
        Indexer, error::IndexerError, run_message_queue_consumer, run_nullifier_spend_listener,
        run_recovery_id_registration_listener,
    },
    message_queue::{DynMessageQueue, MessageQueue, mock_message_queue::MockMessageQueue},
    state_transitions::StateApplicator,
    types::MasterViewSeed,
};
use darkpool_indexer_api::types::message_queue::{
    Message, NullifierSpendMessage, RecoveryIdMessage,
};
use eyre::{OptionExt, Result};
use postgresql_embedded::PostgreSQL;
use renegade_circuit_types::csprng::PoseidonCSPRNG;
use renegade_constants::Scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::IDarkpoolV2Instance;
use test_helpers::types::TestVerbosity;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::{
    CliArgs,
    utils::setup::{
        BASE_TOKEN_DEPLOYMENT_KEY, PERMIT2_DEPLOYMENT_KEY, gen_test_master_view_seed,
        read_deployment, register_test_master_view_seed,
    },
};

/// The arguments passed to every integration test
#[derive(Clone)]
pub struct TestArgs {
    /// The path to the contract deployments file
    pub deployments: PathBuf,
    /// The WS RPC URL of the Anvil node to use for testing
    pub anvil_ws_url: String,
    /// The verbosity with which to run the tests
    pub verbosity: TestVerbosity,
    /// The indexer context, constructed before each test
    pub indexer_context: Option<IndexerContext>,
    /// The Anvil context, constructed once during test harness setup
    pub anvil_context: OnceCell<AnvilContext>,
    /// The first test account's master view seed, which will be pre-allocated
    /// into the indexer
    pub party0_master_view_seed: MasterViewSeed,
    /// The first test account's private key
    pub party0_signer: PrivateKeySigner,
}

/// A container for indexer-specific contextual resources, constructed before
/// each test.
#[derive(Clone)]
pub struct IndexerContext {
    /// The indexer instance to test
    pub indexer: Arc<Indexer>,
    /// The background indexer tasks spawned during setup.
    ///
    /// We store this on the indexer context to prevent the tasks from being
    /// dropped.
    #[allow(unused)]
    pub indexer_tasks: Arc<JoinSet<Result<(), IndexerError>>>,
    /// The local PostgreSQL instance to use for testing
    pub postgres: Arc<PostgreSQL>,
}

/// A container for Anvil-specific contextual resources, constructed during test
/// harness setup.
#[derive(Clone)]
pub struct AnvilContext {
    /// The darkpool client to use for testing
    pub darkpool_client: DarkpoolClient,
    /// The ID of the Anvil snapshot from which to run each test
    pub anvil_snapshot_id: U256,
}

impl TestArgs {
    // --- Indexer Context Helpers --- //

    /// Construct a new indexer context and inject it into the test args
    pub async fn inject_indexer_context(&mut self) -> Result<()> {
        // Set up a test DB client & state applicator
        let (db_client, postgres) = setup_test_db().await?;
        let postgres = Arc::new(postgres);
        let state_applicator = StateApplicator::new(db_client.clone());

        // Set up the mock message queue
        let message_queue = DynMessageQueue::new(MockMessageQueue::default());

        let chain_event_listener = ChainEventListener::new(
            self.darkpool_client().clone(),
            0, // nullifier_start_block
            0, // recovery_id_start_block
            message_queue.clone(),
        );

        // Construct the indexer instance
        let indexer = Arc::new(Indexer {
            db_client,
            state_applicator,
            message_queue,
            darkpool_client: self.darkpool_client().clone(),
            chain_event_listener,
            http_auth_key: None,
        });

        // Register the test account's master view seed into the indexer
        register_test_master_view_seed(&indexer, &self.party0_master_view_seed).await?;

        // Spawn the indexer tasks
        let mut indexer_tasks = JoinSet::new();

        indexer_tasks.spawn(run_message_queue_consumer(indexer.clone()));
        indexer_tasks.spawn(run_nullifier_spend_listener(indexer.clone()));
        indexer_tasks.spawn(run_recovery_id_registration_listener(indexer.clone()));

        let indexer_tasks = Arc::new(indexer_tasks);

        self.indexer_context = Some(IndexerContext { indexer, indexer_tasks, postgres });

        Ok(())
    }

    /// Get a reference to the indexer context, expecting it to be set
    pub fn expect_indexer_context(&self) -> &IndexerContext {
        self.indexer_context.as_ref().unwrap()
    }

    /// Teardown the indexer context, cleaning up the test database and aborting
    /// the indexer tasks
    pub async fn teardown_indexer_context(&mut self) -> Result<()> {
        let mut indexer_context = self.indexer_context.take().unwrap();

        // Abort all the indexer tasks
        let indexer_tasks_mut = Arc::get_mut(&mut indexer_context.indexer_tasks)
            .ok_or_eyre("Multiple references to indexer tasks")?;

        indexer_tasks_mut.abort_all();

        // Close the DB client's pool before dropping the test database
        indexer_context.indexer.db_client.db_pool.close();

        // Clean up the test database
        cleanup_test_db(&indexer_context.postgres).await?;

        Ok(())
    }

    /// Send a message directly to the indexer's message queue
    pub async fn send_message(
        &self,
        message: Message,
        deduplication_id: String,
        message_group: String,
    ) -> Result<()> {
        let indexer_context = self.expect_indexer_context();
        indexer_context
            .indexer
            .message_queue
            .send_message(message, deduplication_id, message_group)
            .await?;

        Ok(())
    }

    /// Send a recovery ID registration message to the indexer's message queue
    pub async fn send_recovery_id_registration_message(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
    ) -> Result<()> {
        let message = Message::RegisterRecoveryId(RecoveryIdMessage { recovery_id, tx_hash });

        let recovery_id_str = recovery_id.to_string();
        self.send_message(message, recovery_id_str.clone(), recovery_id_str).await
    }

    /// Send a nullifier spend message to the indexer's message queue
    pub async fn send_nullifier_spend_message(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
    ) -> Result<()> {
        let message = Message::NullifierSpend(NullifierSpendMessage { nullifier, tx_hash });
        let nullifier_str = nullifier.to_string();
        self.send_message(message, nullifier_str.clone(), nullifier_str).await
    }

    /// Get a reference to the DB client
    pub fn db_client(&self) -> &DbClient {
        let indexer_context = self.expect_indexer_context();
        &indexer_context.indexer.db_client
    }

    // --- Anvil Context Helpers --- //

    /// Get a reference to the anvil context, expecting it to be set
    pub fn expect_anvil_context(&self) -> &AnvilContext {
        self.anvil_context.get().unwrap()
    }

    /// Get a reference to the darkpool client
    pub fn darkpool_client(&self) -> &DarkpoolClient {
        let anvil_context = self.expect_anvil_context();
        &anvil_context.darkpool_client
    }

    /// Get the chain ID of the Anvil node
    pub async fn chain_id(&self) -> Result<u64> {
        let darkpool_client = self.darkpool_client();
        let chain_id = darkpool_client.provider().get_chain_id().await?;
        Ok(chain_id)
    }

    /// Get the darkpool instance
    pub fn darkpool_instance(&self) -> IDarkpoolV2Instance<DynProvider> {
        let darkpool_client = self.darkpool_client();
        darkpool_client.darkpool.clone()
    }

    /// Revert the Anvil node to the snapshot ID stored in the anvil context
    pub async fn revert_anvil_snapshot(&self) -> Result<()> {
        let anvil_context = self.expect_anvil_context();
        let anvil_snapshot_id = anvil_context.anvil_snapshot_id;
        let provider = self.darkpool_client().provider();
        provider.anvil_revert(anvil_snapshot_id).await?;

        Ok(())
    }

    // --- Test Account Helpers --- //

    /// Get the first test account's address
    pub fn party0_address(&self) -> Address {
        self.party0_master_view_seed.owner_address
    }

    /// Get the first test account's private key
    pub fn party0_signer(&self) -> PrivateKeySigner {
        self.party0_signer.clone()
    }

    /// Get the first test account's ID
    pub fn party0_account_id(&self) -> Uuid {
        self.party0_master_view_seed.account_id
    }

    /// Generate the next share stream for the first test account
    pub fn next_party0_share_stream(&mut self) -> PoseidonCSPRNG {
        let share_stream_seed = self.party0_master_view_seed.share_seed_csprng.next().unwrap();
        PoseidonCSPRNG::new(share_stream_seed)
    }

    /// Generate the next recovery stream for the first test account
    pub fn next_party0_recovery_stream(&mut self) -> PoseidonCSPRNG {
        let recovery_stream_seed =
            self.party0_master_view_seed.recovery_seed_csprng.next().unwrap();

        PoseidonCSPRNG::new(recovery_stream_seed)
    }

    // --- Contract Addresses --- //

    /// Get the darkpool contract address
    pub fn darkpool_address(&self) -> Address {
        self.darkpool_client().darkpool_address()
    }

    /// Get the Permit2 contract address
    pub fn permit2_address(&self) -> Result<Address> {
        read_deployment(PERMIT2_DEPLOYMENT_KEY, &self.deployments)
    }

    /// Get the address of the base token
    pub fn base_token_address(&self) -> Result<Address> {
        read_deployment(BASE_TOKEN_DEPLOYMENT_KEY, &self.deployments)
    }
}

impl From<CliArgs> for TestArgs {
    fn from(value: CliArgs) -> Self {
        let party0_signer = PrivateKeySigner::from_str(&value.pkey).unwrap();

        let party0_master_view_seed = gen_test_master_view_seed(&party0_signer);

        Self {
            deployments: value.deployments,
            anvil_ws_url: value.anvil_ws_url,
            verbosity: value.verbosity,
            // The indexer context will be constructed before each test
            indexer_context: None,
            // The anvil context will be constructed during test harness setup
            anvil_context: OnceCell::new(),
            party0_master_view_seed,
            party0_signer,
        }
    }
}
