//! A lightweight worker that periodically samples an estimate of the gas cost
//! for an external match

use std::sync::Arc;

use alloy::primitives::{Address, U256};
use alloy::sol;
use rand::{thread_rng, RngCore};
use renegade_arbitrum_client::client::RenegadeProvider;
use renegade_system_clock::{SystemClock, SystemClockError};
use tokio::sync::RwLock;

use crate::error::AuthServerError;

use super::constants::{
    ESTIMATED_COMPRESSED_CALLDATA_SIZE_BYTES, ESTIMATED_L2_GAS, GAS_COST_SAMPLING_INTERVAL,
    NODE_INTERFACE_ADDRESS,
};

// -------
// | ABI |
// -------

// The ABI for the `NodeInterface` precompile:
// https://docs.arbitrum.io/build-decentralized-apps/nodeinterface/overview
sol! {
    #[sol(rpc)]
    contract NodeInterface {
        function gasEstimateL1Component(address to, bool contractCreation, bytes calldata data) external payable returns (uint64 gasEstimateForL1, uint256 baseFee, uint256 l1BaseFeeEstimate);
    }
}

/// A lightweight worker that periodically samples an estimate of the gas cost
/// for an external match
#[derive(Clone)]
pub struct GasCostSampler {
    /// The latest estimate of the gas cost for an external match
    latest_estimate: Arc<RwLock<U256>>,
    /// An Arbitrum RPC client
    client: RenegadeProvider,
    /// The address of the gas sponsor contract
    gas_sponsor_address: Address,
}

impl GasCostSampler {
    /// Create a new gas cost sampler
    pub async fn new(
        client: RenegadeProvider,
        gas_sponsor_address: Address,
        system_clock: &SystemClock,
    ) -> Result<Self, AuthServerError> {
        let this = Self {
            latest_estimate: Arc::new(RwLock::new(U256::ZERO)),
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

    /// Sample the current L1 & L2 gas prices.
    /// Returns a tuple containing:
    /// - `gas_estimate_for_l1`: the cost in units of L2 gas for including all
    ///   of the provided calldata. Effectively equal to
    ///   `compressed_calldata_size * l1_base_fee_estimate*16 / l2_base_fee`.
    /// - `l2_base_fee`: the cost in wei of a single unit of L2 gas.
    /// - `l1_base_fee_estimate*16`: the cost in wei (on the L2) of including a
    ///   single byte of calldata.
    pub async fn sample_gas_prices(&self) -> Result<(u64, U256, U256), String> {
        // Get the estimate of the L1 gas costs of the transaction.
        // As per https://github.com/OffchainLabs/nitro-contracts/blob/main/src/node-interface/NodeInterface.sol#L102-L103,
        // this doesn't actually simulate the transaction, just estimates L1 portion of
        // gas costs from the calldata size.
        let node_interface = NodeInterface::new(NODE_INTERFACE_ADDRESS, self.client.clone());

        // The arguments to the `gasEstimateL1Component` call are largely irrelevant.
        // Primarily, we're interested in mocking the calldata,
        // which we do so by constructing `ESTIMATED_COMPRESSED_CALLDATA_SIZE_BYTES`
        // random bytes
        let mut data = [0_u8; ESTIMATED_COMPRESSED_CALLDATA_SIZE_BYTES];
        thread_rng().fill_bytes(&mut data);

        let res = node_interface
            .gasEstimateL1Component(
                self.gas_sponsor_address,
                false, // contract_creation
                data.into(),
            )
            .call()
            .await
            .map_err(|e| e.to_string())?;
        let (gas_estimate_for_l1, l2_base_fee, l1_base_fee_estimate) =
            (res.gasEstimateForL1, res.baseFee, res.l1BaseFeeEstimate);

        Ok((gas_estimate_for_l1, l2_base_fee, l1_base_fee_estimate * U256::from(16)))
    }

    /// Estimate the gas cost, in wei, of an external match.
    /// This calculation was taken from https://docs.arbitrum.io/build-decentralized-apps/how-to-estimate-gas
    async fn estimate_external_match_gas_cost(&self) -> Result<(), String> {
        let (gas_estimate_for_l1, l2_base_fee, _) = self.sample_gas_prices().await?;
        let total_gas = U256::from(ESTIMATED_L2_GAS + gas_estimate_for_l1);
        let total_cost = total_gas * l2_base_fee;

        let mut latest_estimate = self.latest_estimate.write().await;
        *latest_estimate = total_cost;

        Ok(())
    }
}
