//! Handlers for executing swaps

use std::sync::Arc;

use ethers::{
    providers::Middleware,
    signers::LocalWallet,
    types::{Address, Eip1559TransactionRequest, TransactionReceipt, U256},
};
use tracing::info;

use crate::helpers::ERC20;

use super::{error::ExecutionClientError, quotes::ExecutionQuote, ExecutionClient};

impl ExecutionClient {
    /// Execute a quoted swap
    pub async fn execute_swap(
        &self,
        quote: ExecutionQuote,
        wallet: LocalWallet,
    ) -> Result<(), ExecutionClientError> {
        // Approve the necessary ERC20 allowance
        self.approve_erc20_allowance(
            quote.sell_token_address,
            quote.to,
            quote.sell_amount,
            &wallet,
        )
        .await?;

        // Execute the swap
        let receipt = self.execute_swap_tx(quote, wallet).await?;
        info!("Swap executed at {}", receipt.transaction_hash);
        Ok(())
    }

    /// Approve an erc20 allowance
    async fn approve_erc20_allowance(
        &self,
        token_address: Address,
        spender: Address,
        amount: U256,
        wallet: &LocalWallet,
    ) -> Result<TransactionReceipt, ExecutionClientError> {
        let client = self.get_signer(wallet.clone());
        let erc20 = ERC20::new(token_address, Arc::new(client));
        let tx = erc20.approve(spender, amount);
        let pending_tx = tx.send().await.map_err(ExecutionClientError::arbitrum)?;

        pending_tx
            .await
            .map_err(ExecutionClientError::arbitrum)?
            .ok_or_else(|| ExecutionClientError::arbitrum("Transaction failed"))
    }

    /// Execute a swap
    async fn execute_swap_tx(
        &self,
        quote: ExecutionQuote,
        wallet: LocalWallet,
    ) -> Result<TransactionReceipt, ExecutionClientError> {
        let client = self.get_signer(wallet.clone());
        let tx = Eip1559TransactionRequest::new()
            .to(quote.to)
            .from(quote.from)
            .value(quote.value)
            .data(quote.data);

        // Send the transaction
        let pending_tx = client
            .send_transaction(tx, None /* block */)
            .await
            .map_err(ExecutionClientError::arbitrum)?;
        pending_tx
            .await
            .map_err(ExecutionClientError::arbitrum)?
            .ok_or_else(|| ExecutionClientError::arbitrum("Transaction failed"))
    }
}
