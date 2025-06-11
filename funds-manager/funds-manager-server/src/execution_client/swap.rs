//! Handlers for executing swaps

use alloy::{
    eips::BlockId,
    hex,
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
use tracing::{info, instrument, warn};

use crate::helpers::IERC20;

use super::{error::ExecutionClientError, ExecutionClient};

/// The factor by which the swap size will be divided when retrying
const SWAP_DECAY_FACTOR: U256 = U256::from_limbs([2, 0, 0, 0]);
/// The minimum amount of USDC that will be attempted to be swapped recursively
const MIN_SWAP_QUOTE_AMOUNT: f64 = 10.0; // 10 USDC
/// The amount to increase an approval by for a swap
///
/// We "over-approve" so that we don't need to re-approve on every swap
const APPROVAL_AMPLIFIER: U256 = U256::from_limbs([4, 0, 0, 0]);
/// The address of the LiFi diamond (same address on Arbitrum One and Base
/// Mainnet), constantized here to simplify approvals
pub const LIFI_DIAMOND_ADDRESS: Address =
    Address::new(hex!("0x1231deb6f5749ef6ce6943a275a1d3e7486f4eae"));

// -----------
// | Helpers |
// -----------

/// Compute the gas cost of a transaction in WEI
fn get_gas_cost(receipt: &TransactionReceipt) -> U256 {
    U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price)
}

// --------------------
// | Execution Client |
// --------------------

impl ExecutionClient {
    /// Construct a swap transaction from an execution quote
    async fn build_swap_tx(
        &self,
        quote: &ExecutionQuote,
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
            .with_input(quote.data.clone())
            .with_max_fee_per_gas(latest_basefee * 2)
            .with_max_priority_fee_per_gas(latest_basefee * 2)
            .with_gas_limit(gas_limit);

        Ok(tx)
    }

    /// Attempt to execute a swap, retrying failed swaps with
    /// decreased quotes down to a minimum trade size.
    ///
    /// Returns the quote, transaction receipt, and cumulative gas cost of all
    /// attempted swaps
    pub async fn swap_immediate_decaying(
        &self,
        chain: Chain,
        mut params: LiFiQuoteParams,
        wallet: PrivateKeySigner,
    ) -> Result<(AugmentedExecutionQuote, TransactionReceipt, U256), ExecutionClientError> {
        let mut cumulative_gas_cost = U256::ZERO;
        loop {
            let augmented_quote = self.get_augmented_quote(params.clone(), chain).await?;

            // Submit the swap
            let client = self.get_signing_provider(wallet.clone());
            let tx = self.build_swap_tx(&augmented_quote.quote, &client).await?;
            let receipt = self.send_tx(tx, &client).await?;
            cumulative_gas_cost += get_gas_cost(&receipt);

            // If the swap succeeds, return
            if receipt.status() {
                return Ok((augmented_quote, receipt, cumulative_gas_cost));
            }

            // Otherwise, decrease the swap size and try again
            warn!("tx ({:#x}) failed, retrying w/ reduced-size quote", receipt.transaction_hash);
            params =
                LiFiQuoteParams { from_amount: params.from_amount / SWAP_DECAY_FACTOR, ..params };
        }
    }

    /// Get an execution quote for a swap
    #[instrument(skip_all)]
    pub async fn get_augmented_quote(
        &self,
        params: LiFiQuoteParams,
        chain: Chain,
    ) -> Result<AugmentedExecutionQuote, ExecutionClientError> {
        let quote = self.get_quote(params).await?;
        let augmented_quote = AugmentedExecutionQuote::new(quote.clone(), chain);

        let quote_amount =
            augmented_quote.get_quote_amount().map_err(ExecutionClientError::parse)?;
        if quote_amount < MIN_SWAP_QUOTE_AMOUNT {
            return Err(ExecutionClientError::custom(format!(
                "Recursive swap amount of {quote_amount} USDC is less than minimum swap amount ({MIN_SWAP_QUOTE_AMOUNT})"
            )));
        }

        Ok(augmented_quote)
    }

    /// Approve an erc20 allowance
    #[instrument(skip(self, wallet))]
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
        let approval_amount = amount * APPROVAL_AMPLIFIER;
        let tx = erc20.approve(spender, approval_amount);
        let pending_tx = tx.send().await.map_err(ExecutionClientError::arbitrum)?;

        let receipt = pending_tx.get_receipt().await.map_err(ExecutionClientError::arbitrum)?;

        info!("Approved erc20 allowance at: {:#x}", receipt.transaction_hash);
        Ok(())
    }
}
