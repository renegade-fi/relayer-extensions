//! Handlers for gas sponsor operations

use std::collections::HashMap;

use alloy::{
    eips::BlockId,
    network::TransactionBuilder,
    providers::Provider,
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::utils::parse_ether;
use alloy_sol_types::SolCall;
use renegade_common::types::token::Token;
use tracing::info;

use crate::{error::FundsManagerError, helpers::fetch_s3_object};

use super::{CustodyClient, DepositWithdrawSource};

// -------------
// | Constants |
// -------------

/// The suffix used to denote the gas sponsor allocation bucket
const ALLOCATION_SPONSOR_BUCKET_SUFFIX: &str = "gas-sponsor-allocation";

/// The key used to denote the gas sponsor allocation object
const ALLOCATION_OBJECT_KEY: &str = "allocation.json";

/// The threshold beneath which we skip refilling gas for the gas sponsor.
/// If the contract's balance deviates from the desired balance by less than
/// this proportion, we skip refilling
const GAS_SPONSOR_REFILL_TOLERANCE: f64 = 0.1; // 10%

/// The ticker used to denote the native ETH allocation
/// in the gas sponsor allocation
const NATIVE_ETH_TICKER: &str = "ETH";

// ---------
// | Types |
// ---------

/// A type alias describing the format of token allocations, namely a map from
/// ticker to amount (in units of whole tokens)
type GasSponsorAllocation = HashMap<String, f64>;

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
    pub(crate) async fn refill_gas_sponsor(&self) -> Result<(), FundsManagerError> {
        // Fetch token allocation from S3
        let allocation = self.fetch_gas_sponsor_allocation().await?;

        self.refill_gas_sponsor_tokens(&allocation).await?;
        self.refill_gas_sponsor_eth(&allocation).await
    }

    /// Fetch the gas sponsor allocation from S3
    async fn fetch_gas_sponsor_allocation(
        &self,
    ) -> Result<GasSponsorAllocation, FundsManagerError> {
        let bucket = format!("{}-{ALLOCATION_SPONSOR_BUCKET_SUFFIX}", self.chain);
        let json_str = fetch_s3_object(&bucket, ALLOCATION_OBJECT_KEY, &self.aws_config).await?;

        // Parse the JSON string to GasSponsorAllocation
        let allocation: GasSponsorAllocation =
            serde_json::from_str(&json_str).map_err(FundsManagerError::parse)?;

        Ok(allocation)
    }

    /// Refill the gas sponsor with ERC-20 tokens for in-kind sponsorship
    async fn refill_gas_sponsor_tokens(
        &self,
        allocation: &GasSponsorAllocation,
    ) -> Result<(), FundsManagerError> {
        let gas_sponsor_address = self.gas_sponsor_address();

        for (ticker, desired_amount) in allocation {
            // Skip the native ETH allocation, that is handled in `refill_gas_sponsor_eth`
            if ticker == NATIVE_ETH_TICKER {
                continue;
            }

            let token = Token::from_ticker_on_chain(ticker, self.chain);

            // Get the gas sponsor's balance of the token
            let bal = self.get_erc20_balance(&token.addr, &gas_sponsor_address).await?;

            if bal < desired_amount * (1.0 - GAS_SPONSOR_REFILL_TOLERANCE) {
                let amount_to_send = desired_amount - bal;
                let receipt = self.send_tokens_to_gas_sponsor(&token.addr, amount_to_send).await?;
                info!(
                    "Sent {amount_to_send} {ticker} from hot wallet to gas sponsor in tx {:#x}",
                    receipt.transaction_hash
                );
            }
        }

        Ok(())
    }

    /// Refill the gas sponsor with ETH
    async fn refill_gas_sponsor_eth(
        &self,
        allocation: &GasSponsorAllocation,
    ) -> Result<(), FundsManagerError> {
        let desired_eth_amount = allocation
            .get(NATIVE_ETH_TICKER)
            .ok_or(FundsManagerError::custom("Gas sponsor allocation missing ETH entry"))?;

        let bal = self.get_ether_balance(&self.gas_sponsor_address()).await?;

        if bal < desired_eth_amount * (1.0 - GAS_SPONSOR_REFILL_TOLERANCE) {
            let amount_to_send = desired_eth_amount - bal;
            let receipt = self.send_eth_to_gas_sponsor(amount_to_send).await?;
            info!(
                "Sent {amount_to_send} ETH from hot wallet to gas sponsor in tx {:#x}",
                receipt.transaction_hash
            );
        }

        Ok(())
    }

    /// Send ERC-20 tokens to the gas sponsor contract
    async fn send_tokens_to_gas_sponsor(
        &self,
        mint: &str,
        amount: f64,
    ) -> Result<TransactionReceipt, FundsManagerError> {
        // Get the quoter hot wallet's private key
        let quoter_wallet = self.get_quoter_hot_wallet().await?;
        let signer = self.get_hot_wallet_private_key(&quoter_wallet.address).await?;

        let bal = self.get_erc20_balance(mint, &quoter_wallet.address).await?;
        if bal < amount {
            return Err(FundsManagerError::custom(format!(
                "quoter hot wallet does not have enough {mint} to cover the refill"
            )));
        }

        let receipt =
            self.erc20_transfer(mint, &self.gas_sponsor_address(), amount, signer).await?;

        Ok(receipt)
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

        let calldata = sol::receiveEthCall {}.abi_encode();

        let amount_units = parse_ether(&amount.to_string()).map_err(FundsManagerError::parse)?;

        let latest_block = client
            .get_block(BlockId::latest())
            .await
            .map_err(FundsManagerError::arbitrum)?
            .ok_or(FundsManagerError::arbitrum("No latest block found".to_string()))?;

        let latest_basefee = latest_block
            .header
            .base_fee_per_gas
            .ok_or(FundsManagerError::arbitrum("No basefee found".to_string()))?
            as u128;

        let tx = TransactionRequest::default()
            .with_input(calldata)
            .with_to(self.gas_sponsor_address)
            .with_value(amount_units)
            .with_gas_price(latest_basefee * 2);

        let pending_tx = client.send_transaction(tx).await.map_err(FundsManagerError::arbitrum)?;

        let receipt = pending_tx.get_receipt().await.map_err(FundsManagerError::arbitrum)?;

        Ok(receipt)
    }
}
