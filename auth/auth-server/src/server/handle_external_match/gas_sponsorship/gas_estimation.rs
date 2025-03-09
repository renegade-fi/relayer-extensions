//! Gas estimation for gas sponsorship

use alloy_primitives::hex;
use ethers::{
    contract::abigen,
    types::{Address, H160, U256},
};
use rand::{thread_rng, RngCore};

use crate::{error::AuthServerError, server::Server};

// -------------
// | Constants |
// -------------

/// The estimated gas cost of L2 execution for an external match.
/// From https://github.com/renegade-fi/renegade/blob/main/workers/api-server/src/http/external_match.rs/#L62
pub const ESTIMATED_L2_GAS: u64 = 4_000_000; // 4m

/// The approximate size in bytes of the calldata for an external match,
/// obtained empirically
const ESTIMATED_CALLDATA_SIZE_BYTES: usize = 8_000;

/// The address of the `NodeInterface` precompile
const NODE_INTERFACE_ADDRESS: Address = H160(hex!("00000000000000000000000000000000000000c8"));

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

impl Server {
    /// Estimate the gas cost, in wei, of an external match.
    /// This calculation was taken from https://docs.arbitrum.io/build-decentralized-apps/how-to-estimate-gas
    pub async fn estimate_external_match_gas_cost(&self) -> Result<U256, AuthServerError> {
        let client = self.arbitrum_client.client();

        // Get the estimate of the L1 gas costs of the transaction.
        // As per https://github.com/OffchainLabs/nitro-contracts/blob/main/src/node-interface/NodeInterface.sol#L102-L103,
        // this doesn't actually simulate the transaction, just estimates L1 portion of
        // gas costs from the calldata size.
        let node_interface = NodeInterface::new(NODE_INTERFACE_ADDRESS, client.clone());

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
            .map_err(AuthServerError::arbitrum_client)?;

        let total_gas = U256::from(ESTIMATED_L2_GAS + gas_estimate_for_l1);
        let total_cost = total_gas * base_fee;

        Ok(total_cost)
    }
}
