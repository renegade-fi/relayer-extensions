//! A lightweight worker that periodically samples an estimate of the gas cost
//! for an external match

use std::sync::Arc;

use ethers::{
    contract::abigen,
    types::{Address, U256},
};
use rand::{thread_rng, RngCore};
use renegade_arbitrum_client::client::MiddlewareStack;
use renegade_system_clock::{SystemClock, SystemClockError};
use tokio::sync::RwLock;

use crate::error::AuthServerError;

use super::constants::{
    ESTIMATED_CALLDATA_SIZE_BYTES, ESTIMATED_L2_GAS, GAS_COST_SAMPLING_INTERVAL,
    NODE_INTERFACE_ADDRESS,
};

// -------
// | ABI |
// -------

// The ABI for the `NodeInterface` precompile:
// https://docs.arbitrum.io/build-decentralized-apps/nodeinterface/overview
abigen!(
    NodeInterface,
    r#"[
        function gasEstimateL1Component(address to, bool contractCreation, bytes calldata data) external payable returns (uint64 gasEstimateForL1, uint256 baseFee, uint256 l1BaseFeeEstimate)
    ]"#
);

/// A lightweight worker that periodically samples an estimate of the gas cost
/// for an external match
#[derive(Clone)]
pub struct GasCostSampler {
    /// The latest estimate of the gas cost for an external match
    latest_estimate: Arc<RwLock<U256>>,
    /// An Arbitrum RPC client
    client: Arc<MiddlewareStack>,
    /// The address of the gas sponsor contract
    gas_sponsor_address: Address,
}

impl GasCostSampler {
    /// Create a new gas cost sampler
    pub async fn new(
        client: Arc<MiddlewareStack>,
        gas_sponsor_address: Address,
        system_clock: &SystemClock,
    ) -> Result<Self, AuthServerError> {
        let this = Self {
            latest_estimate: Arc::new(RwLock::new(U256::zero())),
            client,
            gas_sponsor_address,
        };

        // Sample an initial estimate of the gas cost since the timer will not run
        // until one interval has passed.
        this.estimate_external_match_gas_cost().await.map_err(AuthServerError::gas_cost_sampler)?;

        let this_for_timer = this.clone();

        system_clock
            .add_async_timer(
                "gas-cost-sampler".to_string(),
                GAS_COST_SAMPLING_INTERVAL,
                move || {
                    let this_for_future = this_for_timer.clone();
                    async move { this_for_future.estimate_external_match_gas_cost().await }
                },
            )
            .await
            .map_err(|SystemClockError(e)| AuthServerError::gas_cost_sampler(e))?;

        Ok(this)
    }

    /// Get the latest estimate of the gas cost for an external match
    pub async fn get_latest_estimate(&self) -> U256 {
        *self.latest_estimate.read().await
    }

    /// Estimate the gas cost, in wei, of an external match.
    /// This calculation was taken from https://docs.arbitrum.io/build-decentralized-apps/how-to-estimate-gas
    async fn estimate_external_match_gas_cost(&self) -> Result<(), String> {
        // Get the estimate of the L1 gas costs of the transaction.
        // As per https://github.com/OffchainLabs/nitro-contracts/blob/main/src/node-interface/NodeInterface.sol#L102-L103,
        // this doesn't actually simulate the transaction, just estimates L1 portion of
        // gas costs from the calldata size.
        let node_interface = NodeInterface::new(NODE_INTERFACE_ADDRESS, self.client.clone());

        // The arguments to the `gasEstimateL1Component` call are largely irrelevant.
        // Primarily, we're interested in mocking the calldata,
        // which we do so by constructing `ESTIMATED_CALLDATA_SIZE_BYTES` random bytes,
        // as a pessimistic assumption of the compressibility of the calldata.
        let mut data = [0_u8; ESTIMATED_CALLDATA_SIZE_BYTES];
        thread_rng().fill_bytes(&mut data);

        let (gas_estimate_for_l1, base_fee, _) = node_interface
            .gas_estimate_l1_component(
                self.gas_sponsor_address,
                false, // contract_creation
                data.into(),
            )
            .call()
            .await
            .map_err(|e| e.to_string())?;

        let total_gas = U256::from(ESTIMATED_L2_GAS + gas_estimate_for_l1);
        let total_cost = total_gas * base_fee;

        let mut latest_estimate = self.latest_estimate.write().await;
        *latest_estimate = total_cost;

        Ok(())
    }
}
