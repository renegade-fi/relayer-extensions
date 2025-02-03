//! Handlers for gas sponsor operations

use alloy_sol_types::SolCall;
use ethers::{
    middleware::SignerMiddleware,
    providers::Middleware,
    signers::{LocalWallet, Signer},
    types::{BlockNumber, TransactionRequest},
    utils::parse_ether,
};
use tracing::info;

use crate::error::FundsManagerError;

use super::{CustodyClient, DepositWithdrawSource};

// -------------
// | Constants |
// -------------

/// The threshold beneath which we skip refilling gas for the gas sponsor
///
/// I.e. if the contract's balance is within this amount of the desired fill, we
/// skip refilling
pub const GAS_SPONSOR_REFILL_TOLERANCE: f64 = 0.001; // ETH

// -------
// | ABI |
// -------

// The ABI for gas sponsorship functions
#[allow(clippy::missing_docs_in_private_items)]
mod sol {
    use alloy_sol_types::sol;

    sol! {
        function receiveEth() external payable;
    }
}

impl CustodyClient {
    // ------------
    // | Handlers |
    // ------------

    /// Refill the gas sponsor
    pub(crate) async fn refill_gas_sponsor(&self, fill_to: f64) -> Result<(), FundsManagerError> {
        let gas_sponsor_address = format!("{:#x}", self.gas_sponsor_address);
        let bal = self.get_ether_balance(&gas_sponsor_address).await?;
        if bal + GAS_SPONSOR_REFILL_TOLERANCE < fill_to {
            self.send_eth_to_gas_sponsor(fill_to - bal).await?;
            info!("Refilled gas sponsor to {fill_to} ETH");
        }

        Ok(())
    }

    /// Send ETH to the gas sponsor contract
    async fn send_eth_to_gas_sponsor(&self, amount: f64) -> Result<(), FundsManagerError> {
        // Get the gas hot wallet's private key
        let source = DepositWithdrawSource::Gas.vault_name();
        let gas_wallet = self.get_hot_wallet_by_vault(source).await?;
        let signer = self.get_hot_wallet_private_key(&gas_wallet.address).await?;

        // Check that the gas wallet has enough ETH to cover the refill
        let my_balance = self.get_ether_balance(&gas_wallet.address).await?;
        if my_balance < amount {
            return Err(FundsManagerError::custom(
                "gas wallet does not have enough ETH to cover the refill",
            ));
        }

        // Invoke the `receiveEth` function on the gas sponsor contract
        self.send_receive_eth_tx(amount, signer).await
    }

    /// Send a transaction to the gas sponsor contract to invoke the
    /// `receiveEth` function
    async fn send_receive_eth_tx(
        &self,
        amount: f64,
        signer: LocalWallet,
    ) -> Result<(), FundsManagerError> {
        let wallet = signer.with_chain_id(self.chain_id);
        let provider = self.get_rpc_provider()?;
        let client = SignerMiddleware::new(provider, wallet);

        let calldata = sol::receiveEthCall {}.abi_encode();

        let amount_units = parse_ether(amount.to_string()).map_err(FundsManagerError::parse)?;

        let latest_block = client
            .get_block(BlockNumber::Latest)
            .await
            .map_err(FundsManagerError::arbitrum)?
            .ok_or(FundsManagerError::arbitrum("No latest block found".to_string()))?;

        let latest_basefee = latest_block
            .base_fee_per_gas
            .ok_or(FundsManagerError::arbitrum("No basefee found".to_string()))?;

        let tx = TransactionRequest::new()
            .data(calldata)
            .to(self.gas_sponsor_address)
            .value(amount_units)
            .gas_price(latest_basefee * 2);

        info!("Sending {amount} ETH to the gas sponsor contract");

        let pending_tx =
            client.send_transaction(tx, None).await.map_err(FundsManagerError::arbitrum)?;

        pending_tx
            .await
            .map_err(FundsManagerError::arbitrum)?
            .ok_or_else(|| FundsManagerError::arbitrum("Transaction failed".to_string()))?;

        Ok(())
    }
}
