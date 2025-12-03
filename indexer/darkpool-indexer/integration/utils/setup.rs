//! Utilities for setting up the integration tests

use std::{fs, path::Path, str::FromStr};

use alloy::{
    primitives::{Address, U256},
    providers::{DynProvider, Provider, ProviderBuilder, WsConnect, ext::AnvilApi},
    signers::local::PrivateKeySigner,
};
use darkpool_indexer::{
    chain_event_listener::ChainEventListener,
    darkpool_client::DarkpoolClient,
    db::test_utils::setup_test_db,
    indexer::Indexer,
    message_queue::{DynMessageQueue, mock_message_queue::MockMessageQueue},
    state_transitions::{StateApplicator, StateTransition},
    types::MasterViewSeed,
};
use darkpool_indexer_api::types::message_queue::MasterViewSeedMessage;
use eyre::{Result, eyre};
use postgresql_embedded::PostgreSQL;
use rand::thread_rng;
use renegade_constants::Scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::IDarkpoolV2Instance;
use serde_json::Value;
use tokio::runtime::Handle;
use uuid::Uuid;

// -------------
// | Constants |
// -------------

/// The deployments file key for the Darkpool proxy contract
const DARKPOOL_PROXY_DEPLOYMENT_KEY: &str = "DarkpoolProxy";
/// The deployments file key for the base token contract
pub(crate) const BASE_TOKEN_DEPLOYMENT_KEY: &str = "BaseToken";
/// The deployments file key for the quote token contract
const QUOTE_TOKEN_DEPLOYMENT_KEY: &str = "QuoteToken";
/// The deployments file key for the WETH contract
const WETH_DEPLOYMENT_KEY: &str = "Weth";
/// The deployments file key for the Permit2 contract
pub(crate) const PERMIT2_DEPLOYMENT_KEY: &str = "Permit2";

// -----------
// | Helpers |
// -----------

/// Construct a indexer instance for integration testing
pub async fn build_test_indexer(
    anvil_ws_url: &str,
    wallet: PrivateKeySigner,
    deployments_path: &Path,
) -> Result<(Indexer, PostgreSQL)> {
    // Set up a test DB client & state applicator
    let (db_client, postgres) = setup_test_db().await?;
    let state_applicator = StateApplicator::new(db_client.clone());

    // Set up the mock message queue
    let message_queue = DynMessageQueue::new(MockMessageQueue::default());

    // Set up the darkpool client
    let darkpool_client: DarkpoolClient =
        build_test_darkpool_client(anvil_ws_url, wallet, deployments_path).await?;

    let chain_event_listener = ChainEventListener::new(
        darkpool_client.clone(),
        0, // nullifier_start_block
        0, // recovery_id_start_block
        message_queue.clone(),
    );

    let indexer = Indexer {
        db_client,
        state_applicator,
        message_queue,
        darkpool_client,
        chain_event_listener,
        http_auth_key: None,
    };

    Ok((indexer, postgres))
}

/// Construct a test darkpool client, targeting a local Anvil node w/ the
/// darkpool contracts deployed
async fn build_test_darkpool_client(
    anvil_ws_url: &str,
    wallet: PrivateKeySigner,
    deployments_path: &Path,
) -> Result<DarkpoolClient> {
    let wallet_address = wallet.address();

    let ws = WsConnect::new(anvil_ws_url);
    let ws_provider = ProviderBuilder::new().wallet(wallet).connect_ws(ws).await?.erased();

    fund_test_wallet(&ws_provider, wallet_address, deployments_path).await?;

    let darkpool_address = read_deployment(DARKPOOL_PROXY_DEPLOYMENT_KEY, deployments_path)?;

    let darkpool = IDarkpoolV2Instance::new(darkpool_address, ws_provider);
    Ok(DarkpoolClient::new(darkpool))
}

/// Fund the test wallet with the deployed mock ERC20s, and approve the Permit2
/// contract as a spender
async fn fund_test_wallet(
    provider: &DynProvider,
    wallet_address: Address,
    deployments_path: &Path,
) -> Result<()> {
    let base_token_addr = read_deployment(BASE_TOKEN_DEPLOYMENT_KEY, deployments_path)?;
    let quote_token_addr = read_deployment(QUOTE_TOKEN_DEPLOYMENT_KEY, deployments_path)?;
    let weth_addr = read_deployment(WETH_DEPLOYMENT_KEY, deployments_path)?;

    provider.anvil_deal_erc20(wallet_address, base_token_addr, U256::MAX).await?;
    provider.anvil_deal_erc20(wallet_address, quote_token_addr, U256::MAX).await?;
    provider.anvil_deal_erc20(wallet_address, weth_addr, U256::MAX).await?;

    let permit2_addr = read_deployment(PERMIT2_DEPLOYMENT_KEY, deployments_path)?;

    provider
        .anvil_set_erc20_allowance(wallet_address, permit2_addr, base_token_addr, U256::MAX)
        .await?;

    provider
        .anvil_set_erc20_allowance(wallet_address, permit2_addr, quote_token_addr, U256::MAX)
        .await?;

    provider.anvil_set_erc20_allowance(wallet_address, permit2_addr, weth_addr, U256::MAX).await?;

    Ok(())
}

/// Read an address from the deployments.json file
///
/// Returns the address for the given key, or an error if not found
pub fn read_deployment(key: &str, deployments_path: &Path) -> Result<Address> {
    // Read the deployments file
    let content = fs::read_to_string(deployments_path)?;
    let json: Value = serde_json::from_str(&content)?;

    // Get the address string
    let addr_str = json
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre!("Key {} not found in deployments file", key))?;

    // Parse into Address
    let address = Address::from_str(addr_str)?;
    Ok(address)
}

/// Generate a master view seed for the given test wallet
pub fn gen_test_master_view_seed(test_wallet: &PrivateKeySigner) -> MasterViewSeed {
    let account_id = Uuid::new_v4();
    let address = test_wallet.address();
    let seed = Scalar::random(&mut thread_rng());

    MasterViewSeed::new(account_id, address, seed)
}

/// Register the test account's master view seed into the indexer.
///
/// We do this by applying the state transition directly, bypassing the message
/// queue and omitting side effects like triggering a backfill.
pub async fn register_test_master_view_seed(
    indexer: &Indexer,
    master_view_seed: &MasterViewSeed,
) -> Result<()> {
    let account_id = master_view_seed.account_id;
    let owner_address = master_view_seed.owner_address;
    let seed = master_view_seed.seed;

    let transition = StateTransition::RegisterMasterViewSeed(MasterViewSeedMessage {
        account_id,
        owner_address,
        seed,
    });

    indexer.state_applicator.apply_state_transition(transition).await?;

    Ok(())
}

/// Run a future synchronously in the current tokio runtime.
///
/// The future is expected to return an `eyre::Result` which gets unwrapped and
/// returned.
pub fn run_blocking_current<F, T>(fut: F) -> T
where
    F: Future<Output = Result<T>>,
{
    Handle::current().block_on(fut).unwrap()
}
