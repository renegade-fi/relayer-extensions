//! Handlers for executing swaps

use alloy::{
    eips::BlockId,
    network::TransactionBuilder,
    providers::Provider,
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{Address, U256};
use funds_manager_api::{quoters::ExecutionQuote, u256_try_into_u64};
use tracing::info;

use crate::helpers::IERC20;

use super::{error::ExecutionClientError, ExecutionClient};

impl ExecutionClient {
    /// Execute a quoted swap
    pub async fn execute_swap(
        &self,
        quote: ExecutionQuote,
        wallet: &PrivateKeySigner,
    ) -> Result<TransactionReceipt, ExecutionClientError> {
        // Execute the swap
        let receipt = self.execute_swap_tx(quote, wallet).await?;
        let tx_hash = receipt.transaction_hash;
        info!("Swap executed at {tx_hash:#x}");
        Ok(receipt)
    }

    /// Execute a swap
    async fn execute_swap_tx(
        &self,
        quote: ExecutionQuote,
        wallet: &PrivateKeySigner,
    ) -> Result<TransactionReceipt, ExecutionClientError> {
        let client = self.get_signing_provider(wallet.clone());

        // Set approval for the sell token
        self.approve_erc20_allowance(quote.sell_token_address, quote.to, quote.sell_amount, wallet)
            .await?;

        let latest_block = client
            .get_block(BlockId::latest())
            .await
            .map_err(ExecutionClientError::arbitrum)?
            .ok_or(ExecutionClientError::arbitrum("No latest block found"))?;

        let latest_basefee = latest_block
            .header
            .base_fee_per_gas
            .ok_or(ExecutionClientError::arbitrum("No basefee found"))?
            as u128;

        let gas_limit =
            u256_try_into_u64(quote.gas_limit).map_err(ExecutionClientError::arbitrum)?;

        let tx = TransactionRequest::default()
            .with_to(quote.to)
            .with_from(quote.from)
            .with_value(quote.value)
            .with_input(quote.data)
            .with_max_fee_per_gas(latest_basefee * 2)
            .with_max_priority_fee_per_gas(latest_basefee * 2)
            .with_gas_limit(gas_limit);

        // Send the transaction
        let pending_tx =
            client.send_transaction(tx).await.map_err(ExecutionClientError::arbitrum)?;

        let receipt = pending_tx.get_receipt().await.map_err(ExecutionClientError::arbitrum)?;

        if !receipt.status() {
            let error_msg = format!("tx ({:#x}) failed", receipt.transaction_hash);
            return Err(ExecutionClientError::arbitrum(error_msg));
        }

        Ok(receipt)
    }

    /// Approve an erc20 allowance
    async fn approve_erc20_allowance(
        &self,
        token_address: Address,
        spender: Address,
        amount: U256,
        wallet: &PrivateKeySigner,
    ) -> Result<(), ExecutionClientError> {
        let client = self.get_signing_provider(wallet.clone());
        let erc20 = IERC20::new(token_address, client);

        // First, check if the allowance is already sufficient
        let allowance = erc20
            .allowance(wallet.address(), spender)
            .call()
            .await
            .map_err(ExecutionClientError::arbitrum)?;

        if allowance >= amount {
            info!("Already approved erc20 allowance for {spender:#x}");
            return Ok(());
        }

        // Otherwise, approve the allowance
        let tx = erc20.approve(spender, amount);
        let pending_tx = tx.send().await.map_err(ExecutionClientError::arbitrum)?;

        let receipt = pending_tx.get_receipt().await.map_err(ExecutionClientError::arbitrum)?;

        info!("Approved erc20 allowance at: {:#x}", receipt.transaction_hash);
        Ok(())
    }
}
