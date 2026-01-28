//! Arbitrum specific gas oracle contract methods

use alloy::primitives::{Address, U256};
use alloy::providers::DynProvider;
use alloy::sol;
use alloy_primitives::{FixedBytes, hex};

use crate::server::helpers::u64_to_u256;

use super::GasPriceEstimation;

/// The address of the NodeInterface precompile
pub const NODE_INTERFACE_ADDRESS: Address =
    Address(FixedBytes(hex!("00000000000000000000000000000000000000c8")));

// The ABI for the `NodeInterface` precompile:
// https://docs.arbitrum.io/build-decentralized-apps/nodeinterface/overview
sol! {
    #[sol(rpc)]
    contract NodeInterface {
        function gasEstimateL1Component(address to, bool contractCreation, bytes calldata data) external payable returns (uint64 gasEstimateForL1, uint256 baseFee, uint256 l1BaseFeeEstimate);
    }
}

/// Estimate the L1 gas component for a transaction with the given calldata
///
/// Returns a tuple containing:
/// - `gas_estimate_for_l1`: the cost in units of L2 gas for including all of
///   the provided calldata. Effectively equal to `compressed_calldata_size *
///   l1_base_fee_estimate*16 / l2_base_fee`.
/// - `l2_base_fee`: the cost in wei of a single unit of L2 gas.
/// - `l1_data_fee`: the cost in wei (on the L2) of including a single byte of
///   calldata (l1_base_fee_estimate*16).
pub async fn estimate_l1_gas_component(
    provider: DynProvider,
    to: Address,
    data: Vec<u8>,
) -> Result<GasPriceEstimation, String> {
    let node_interface = NodeInterface::new(NODE_INTERFACE_ADDRESS, provider);

    // As per https://github.com/OffchainLabs/nitro-contracts/blob/main/src/node-interface/NodeInterface.sol#L102-L103,
    // this doesn't actually simulate the transaction, just estimates L1 portion of
    // gas costs from the calldata size.
    let res = node_interface
        .gasEstimateL1Component(
            to,
            false, // contract_creation
            data.into(),
        )
        .call()
        .await
        .map_err(|e| e.to_string())?;

    let (gas_estimate_for_l1, l2_base_fee, l1_base_fee_estimate) =
        (res.gasEstimateForL1, res.baseFee, res.l1BaseFeeEstimate);

    Ok(GasPriceEstimation {
        gas_estimate_for_l1: u64_to_u256(gas_estimate_for_l1),
        l2_base_fee,
        l1_data_fee: l1_base_fee_estimate * U256::from(16),
    })
}
