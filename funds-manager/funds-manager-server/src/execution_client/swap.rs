//! Handlers for executing swaps

use alloy::{
    eips::BlockId,
    network::TransactionBuilder,
    providers::{DynProvider, Provider},
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{Address, U256};
use funds_manager_api::{
    quoters::{AugmentedExecutionQuote, ExecutionQuote, LiFiQuoteParams},
    u256_try_into_u64,
};
use renegade_common::types::chain::Chain;
use tracing::{error, info, warn};

use crate::helpers::IERC20;

use super::{error::ExecutionClientError, ExecutionClient};

/// The minimum amount of USDC that will be attempted to be swapped recursively
const MIN_SWAP_QUOTE_AMOUNT: f64 = 10.0; // 10 USDC

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

        let tx = self.build_swap_tx(quote, &client).await?;

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

    /// Construct a swap transaction from an execution quote
    async fn build_swap_tx(
        &self,
        quote: ExecutionQuote,
        client: &DynProvider,
    ) -> Result<TransactionRequest, ExecutionClientError> {
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

        Ok(tx)
    }

    /// Attempt to execute a swap, recursively retrying failed swaps with
    /// half-sized quotes down to a minimum trade size.
    pub async fn swap_immediate_recursive(
        &self,
        chain: Chain,
        params: LiFiQuoteParams,
        wallet: PrivateKeySigner,
    ) -> Result<Vec<(AugmentedExecutionQuote, TransactionReceipt)>, ExecutionClientError> {
        let quote = self.get_quote(params.clone()).await?;
        let augmented_quote = AugmentedExecutionQuote::new(quote.clone(), chain);

        let quote_amount =
            augmented_quote.get_quote_amount().map_err(ExecutionClientError::parse)?;

        if quote_amount < MIN_SWAP_QUOTE_AMOUNT {
            return Err(ExecutionClientError::custom(format!(
                "Recursive swap amount of {quote_amount} USDC is less than minimum swap amount ({MIN_SWAP_QUOTE_AMOUNT})"
            )));
        }

        let client = self.get_signing_provider(wallet.clone());

        let tx = self.build_swap_tx(quote, &client).await?;

        // Send the transaction
        let pending_tx =
            client.send_transaction(tx).await.map_err(ExecutionClientError::arbitrum)?;

        let receipt = pending_tx.get_receipt().await.map_err(ExecutionClientError::arbitrum)?;

        if !receipt.status() {
            warn!("tx ({:#x}) failed, retrying w/ half-sized quotes", receipt.transaction_hash);
            return Box::pin(self.swap_half_size_quotes(chain, params, wallet)).await;
        }

        Ok(vec![(augmented_quote, receipt)])
    }

    /// Attempt to execute a swap across two half-sized quotes
    async fn swap_half_size_quotes(
        &self,
        chain: Chain,
        original_params: LiFiQuoteParams,
        wallet: PrivateKeySigner,
    ) -> Result<Vec<(AugmentedExecutionQuote, TransactionReceipt)>, ExecutionClientError> {
        let half_size_params = LiFiQuoteParams {
            from_amount: original_params.from_amount / U256::from(2),
            ..original_params
        };

        let mut results = vec![];

        let first_half_results =
            self.swap_immediate_recursive(chain, half_size_params.clone(), wallet.clone()).await;

        let second_half_results =
            self.swap_immediate_recursive(chain, half_size_params, wallet).await;

        match first_half_results {
            Ok(first_half_results) => results.extend(first_half_results),
            Err(e) => error!("Failed to execute first half of swap: {e}"),
        }
        match second_half_results {
            Ok(second_half_results) => results.extend(second_half_results),
            Err(e) => error!("Failed to execute second half of swap: {e}"),
        }

        Ok(results)
    }

    /// Approve an erc20 allowance
    pub(crate) async fn approve_erc20_allowance(
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
