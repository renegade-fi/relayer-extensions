//! Base specific gas oracle contract methods

use GasOracle::GasOracleInstance;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::{hex, sol};
use alloy_primitives::FixedBytes;
use renegade_darkpool_client::client::RenegadeProvider;

use super::GasPriceEstimation;

/// The address of the gas oracle contract on Base
pub const GAS_ORACLE_ADDRESS: Address =
    Address(FixedBytes(hex!("0x420000000000000000000000000000000000000F")));

sol! {
    #[sol(rpc)]
    contract GasOracle {
        function l1BaseFee() external view returns (uint256);
        function gasPrice() external view returns (uint256);
        function getL1GasUsed(bytes data) external view returns (uint256);
    }
}

/// Estimate the L1 gas component for a transaction on Base
pub async fn estimate_l1_gas_component(
    provider: RenegadeProvider,
    _to: Address,
    data: Vec<u8>,
) -> Result<GasPriceEstimation, String> {
    // Get the gas price directly from the RPC. This isn't exactly the basefee,
    // but is a sufficient approximation that keeps our RPC costs low.
    // We do this instead of calling `gasPrice` on the oracle, as that method
    // has proven unreliable (i.e., returns 0).
    let gas_price = provider.get_gas_price().await.map_err(|e| e.to_string())?;
    let l2_base_fee = U256::from(gas_price);

    // Sample values from the gas oracle contract
    let gas_oracle = GasOracleInstance::new(GAS_ORACLE_ADDRESS, provider);

    let l1_base_fee = gas_oracle.l1BaseFee().call().await.map_err(|e| e.to_string())?;
    let l1_gas_used =
        gas_oracle.getL1GasUsed(data.into()).call().await.map_err(|e| e.to_string())?;

    Ok(GasPriceEstimation {
        gas_estimate_for_l1: l1_gas_used,
        l2_base_fee,
        // Ethereum L1 charges 16 gas per non-zero byte of calldata
        l1_data_fee: l1_base_fee * U256::from(16),
    })
}
