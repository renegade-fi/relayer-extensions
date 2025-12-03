//! Defines the arguments passed to every test

use std::{path::PathBuf, str::FromStr};

use alloy::{
    primitives::{Address, U256},
    providers::{DynProvider, Provider, ext::AnvilApi},
    signers::local::PrivateKeySigner,
};
use darkpool_indexer::{indexer::Indexer, types::MasterViewSeed};
use eyre::Result;
use postgresql_embedded::PostgreSQL;
use renegade_circuit_types::csprng::PoseidonCSPRNG;
use renegade_solidity_abi::v2::IDarkpoolV2::IDarkpoolV2Instance;

use crate::{
    CliArgs,
    utils::setup::{
        BASE_TOKEN_DEPLOYMENT_KEY, PERMIT2_DEPLOYMENT_KEY, build_test_indexer,
        gen_test_master_view_seed, read_deployment, register_test_master_view_seed, run_blocking,
    },
};

/// The arguments passed to every integration test
#[derive(Clone)]
pub struct TestArgs {
    /// The path to the contract deployments file
    deployments: PathBuf,
    /// The indexer instance to test
    indexer: Indexer,
    /// The local PostgreSQL instance to use for testing
    postgres: PostgreSQL,
    /// The ID of the Anvil snapshot from which to run each test
    anvil_snapshot_id: U256,
    /// The first test account's master view seed, which will be pre-allocated
    /// into the indexer
    party0_master_view_seed: MasterViewSeed,
    /// The first test account's private key
    party0_signer: PrivateKeySigner,
}

impl TestArgs {
    // --- RPC Client Helpers --- //
    /// Get the chain ID of the Anvil node
    pub async fn chain_id(&self) -> Result<u64> {
        let chain_id = self.indexer.darkpool_client.provider().get_chain_id().await?;
        Ok(chain_id)
    }

    /// Get the darkpool instance
    pub fn darkpool_instance(&self) -> IDarkpoolV2Instance<DynProvider> {
        self.indexer.darkpool_client.darkpool.clone()
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
        self.indexer.darkpool_client.darkpool_address()
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
        run_blocking(async {
            let party0_signer = PrivateKeySigner::from_str(&value.pkey)?;

            let (indexer, postgres) =
                build_test_indexer(&value.anvil_ws_url, party0_signer.clone(), &value.deployments)
                    .await?;

            let anvil_snapshot_id = indexer.darkpool_client.provider().anvil_snapshot().await?;

            let party0_master_view_seed = gen_test_master_view_seed(&party0_signer);
            register_test_master_view_seed(&indexer, &party0_master_view_seed).await?;

            let test_args = Self {
                deployments: value.deployments,
                indexer,
                postgres,
                anvil_snapshot_id,
                party0_master_view_seed,
                party0_signer,
            };

            Ok::<_, eyre::Report>(test_args)
        })
    }
}
