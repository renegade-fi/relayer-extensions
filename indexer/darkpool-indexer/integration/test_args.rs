//! Defines the arguments passed to every test

use std::{cell::OnceCell, path::PathBuf, str::FromStr, sync::Arc};

use alloy::{
    primitives::{Address, TxHash, U256},
    providers::{DynProvider, Provider, ext::AnvilApi},
    signers::local::PrivateKeySigner,
};
use darkpool_indexer::{
    db::client::DbClient,
    indexer::{Indexer, error::IndexerError},
    message_queue::MessageQueue,
    types::MasterViewSeed,
};
use darkpool_indexer_api::types::message_queue::{Message, RecoveryIdMessage};
use eyre::Result;
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
        read_deployment,
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
    /// The test context, constructed during test harness setup
    pub test_context: OnceCell<TestContext>,
    /// The first test account's master view seed, which will be pre-allocated
    /// into the indexer
    pub party0_master_view_seed: MasterViewSeed,
    /// The first test account's private key
    pub party0_signer: PrivateKeySigner,
}

/// A container for resources used by all integration tests
/// which are constructed during test harness setup
#[derive(Clone)]
pub struct TestContext {
    /// The indexer instance to test
    pub indexer: Arc<Indexer>,
    /// The background indexer tasks spawned during setup.
    ///
    /// We store this on the test context to prevent the tasks from being
    /// dropped.
    #[allow(unused)]
    pub indexer_tasks: Arc<JoinSet<Result<(), IndexerError>>>,
    /// The local PostgreSQL instance to use for testing
    pub postgres: Arc<PostgreSQL>,
    /// The ID of the Anvil snapshot from which to run each test
    pub anvil_snapshot_id: U256,
}

impl TestArgs {
    // --- Test Context Helpers --- //

    /// Get a reference to the test context, expecting it to be set
    pub fn expect_test_context(&self) -> &TestContext {
        self.test_context.get().unwrap()
    }

    // --- Direct Indexer Access Helpers --- //

    /// Send a message directly to the indexer's message queue
    pub async fn send_message(
        &self,
        message: Message,
        deduplication_id: String,
        message_group: String,
    ) -> Result<()> {
        let test_context = self.expect_test_context();
        test_context
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

    /// Get a reference to the DB client
    pub fn db_client(&self) -> &DbClient {
        let test_context = self.expect_test_context();
        &test_context.indexer.db_client
    }

    // --- RPC Client Helpers --- //

    /// Get the chain ID of the Anvil node
    pub async fn chain_id(&self) -> Result<u64> {
        let test_context = self.expect_test_context();
        let chain_id = test_context.indexer.darkpool_client.provider().get_chain_id().await?;
        Ok(chain_id)
    }

    /// Get the darkpool instance
    pub fn darkpool_instance(&self) -> IDarkpoolV2Instance<DynProvider> {
        let test_context = self.expect_test_context();
        test_context.indexer.darkpool_client.darkpool.clone()
    }

    /// Revert the Anvil node to the snapshot ID stored in the test context
    pub async fn revert_anvil_snapshot(&self) -> Result<()> {
        let test_context = self.expect_test_context();
        let anvil_snapshot_id = test_context.anvil_snapshot_id;
        let provider = test_context.indexer.darkpool_client.provider();
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
        let test_context = self.expect_test_context();
        test_context.indexer.darkpool_client.darkpool_address()
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
            // The test context will be constructed during test harness setup
            test_context: OnceCell::new(),
            party0_master_view_seed,
            party0_signer,
        }
    }
}
