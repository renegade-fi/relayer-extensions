//! Handlers for gas sponsor operations

use alloy::{
    eips::BlockId,
    network::TransactionBuilder,
    providers::Provider,
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::utils::parse_ether;
use alloy_sol_types::SolCall;
use renegade_common::types::{
    chain::Chain,
    token::{get_all_tokens, Token},
};
use tracing::{error, info};

use crate::error::FundsManagerError;

use super::{CustodyClient, DepositWithdrawSource};

// -------------
// | Constants |
// -------------

/// The desired USDC value of the gas sponsor's reserves in each token
pub const DESIRED_GAS_SPONSORSHIP_RESERVE_VALUE: f64 = 50.0;

/// The minimum USDC value of a token transfer to the gas sponsor
const MIN_TRANSFER_VALUE: f64 = 10.0;

/// The factor by which to reduce the amount of a token to send to the gas
/// sponsor when we are sending the entire hot wallet balance of the token
pub const MAX_REFILL_REDUCTION_FACTOR: f64 = 0.9999;

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

    /// Gets the tokens which the gas sponsor needs to be refilled for.
    ///
    /// Returns a vector of (token, refill_amount).
    pub async fn get_tokens_needing_refill(&self) -> Result<Vec<(Token, f64)>, FundsManagerError> {
        let gas_sponsor_address = self.gas_sponsor_address();

        let mut tokens = Vec::new();

        let all_tokens_on_chain = get_all_tokens().into_iter().filter(|t| t.chain == self.chain);

        for token in all_tokens_on_chain {
            // Get the gas sponsor's balance of the token
            let price = self.price_reporter.get_price(&token.addr, self.chain).await?;
            let bal = self.get_erc20_balance(&token.addr, &gas_sponsor_address).await?;
            let bal_value = bal * price;

            if bal_value < DESIRED_GAS_SPONSORSHIP_RESERVE_VALUE - MIN_TRANSFER_VALUE {
                let refill_value = DESIRED_GAS_SPONSORSHIP_RESERVE_VALUE - bal_value;
                let refill_amount = refill_value / price;

                let ticker = token.get_ticker().unwrap_or(token.get_addr());
                info!("Gas sponsor needs {refill_amount} (${refill_value}) of {ticker}");

                tokens.push((token, refill_amount));
            }
        }

        Ok(tokens)
    }

    /// Refill the gas sponsor with ETH
    pub async fn refill_gas_sponsor_eth(&self) -> Result<(), FundsManagerError> {
        let price = self.price_reporter.get_eth_price().await?;
        let bal = self.get_ether_balance(&self.gas_sponsor_address()).await?;
        let bal_value = bal * price;

        if bal_value < DESIRED_GAS_SPONSORSHIP_RESERVE_VALUE - MIN_TRANSFER_VALUE {
            let refill_value = DESIRED_GAS_SPONSORSHIP_RESERVE_VALUE - bal_value;
            let refill_amount = refill_value / price;

            match self.send_eth_to_gas_sponsor(refill_amount).await {
                Ok(TransactionReceipt { transaction_hash: tx, .. }) => {
                    info!("Sent {refill_amount} ETH from hot wallet to gas sponsor in tx {tx:#x}");
                },
                Err(e) => {
                    error!("Failed to send ETH to gas sponsor, skipping: {e}");
                },
            }
        }

        Ok(())
    }

    /// Send the given amount of the ERC-20 to the gas sponsor contract
    pub async fn send_token_to_gas_sponsor(
        &self,
        token: &Token,
        amount: f64,
        quoter_wallet: PrivateKeySigner,
    ) -> Result<(), FundsManagerError> {
        let ticker = token.get_ticker().unwrap_or(token.get_addr());
        let mint = &token.addr;

        let bal =
            self.get_erc20_balance(&token.get_addr(), &quoter_wallet.address().to_string()).await?;

        let send_amount = if bal <= amount {
            let send_amount = bal * MAX_REFILL_REDUCTION_FACTOR;
            info!("Hot wallet has less than the desired balance of {ticker}, sending {send_amount} {ticker}");
            send_amount
        } else {
            amount
        };

        let price = self.price_reporter.get_price(mint, self.chain).await?;
        let value = send_amount * price;

        if value < MIN_TRANSFER_VALUE {
            return Err(FundsManagerError::custom(format!(
                "Attempted transfer of ${value} of {ticker} to gas sponsor is below ${MIN_TRANSFER_VALUE} minimum"
            )));
        }

        let receipt = self
            .erc20_transfer(mint, &self.gas_sponsor_address(), send_amount, quoter_wallet)
            .await?;

        info!(
            "Sent {send_amount} {ticker} from hot wallet to gas sponsor in tx {:#x}",
            receipt.transaction_hash
        );

        Ok(())
    }

    /// Send ETH to the gas sponsor contract
    async fn send_eth_to_gas_sponsor(
        &self,
        amount: f64,
    ) -> Result<TransactionReceipt, FundsManagerError> {
        // Get the gas hot wallet's private key
        let source = DepositWithdrawSource::Gas.vault_name(self.chain);
        let gas_wallet = self.get_hot_wallet_by_vault(&source).await?;
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
        signer: PrivateKeySigner,
    ) -> Result<TransactionReceipt, FundsManagerError> {
        let client = self.get_signing_provider(signer);
        let calldata = self.get_eth_transfer_calldata();
        let amount_units = parse_ether(&amount.to_string()).map_err(FundsManagerError::parse)?;

        let latest_block = client
            .get_block(BlockId::latest())
            .await
            .map_err(FundsManagerError::on_chain)?
            .ok_or(FundsManagerError::on_chain("No latest block found".to_string()))?;

        let latest_basefee = latest_block
            .header
            .base_fee_per_gas
            .ok_or(FundsManagerError::on_chain("No basefee found".to_string()))?
            as u128;

        let tx = TransactionRequest::default()
            .with_input(calldata)
            .with_to(self.gas_sponsor_address)
            .with_value(amount_units)
            .with_gas_price(latest_basefee * 2);

        let pending_tx = client.send_transaction(tx).await.map_err(FundsManagerError::on_chain)?;
        let receipt = pending_tx.get_receipt().await.map_err(FundsManagerError::on_chain)?;
        Ok(receipt)
    }

    /// Get the ETH transfer calldata for the gas sponsor
    fn get_eth_transfer_calldata(&self) -> Vec<u8> {
        match self.chain {
            Chain::ArbitrumSepolia | Chain::ArbitrumOne => sol::receiveEthCall {}.abi_encode(),
            // Solidity implementations use the `receive` fallback function to receive ETH
            // No calldata is needed here
            Chain::BaseSepolia | Chain::BaseMainnet => Vec::new(),
            _ => {
                panic!("transferring eth is not supported on {:?}", self.chain);
            },
        }
    }
}
