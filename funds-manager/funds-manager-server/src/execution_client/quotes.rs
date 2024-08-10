//! Client methods for fetching quotes and prices from the execution venue

use std::{str::FromStr, sync::Arc};

use ethers::{
    signers::{LocalWallet, Signer},
    types::{Address, U256},
};
use funds_manager_api::quoters::ExecutionQuote;
use serde::Deserialize;
use tracing::info;

use crate::helpers::ERC20;

use super::{error::ExecutionClientError, ExecutionClient};

/// The price endpoint
const PRICE_ENDPOINT: &str = "swap/v1/price";
/// The quote endpoint
const QUOTE_ENDPOINT: &str = "swap/v1/quote";

/// The buy token url param
const BUY_TOKEN: &str = "buyToken";
/// The sell token url param
const SELL_TOKEN: &str = "sellToken";
/// The sell amount url param
const SELL_AMOUNT: &str = "sellAmount";
/// The taker address url param
const TAKER_ADDRESS: &str = "takerAddress";

/// The 0x exchange proxy contract address
///
/// TODO: This is the same across _most_ chains, but if we wish to support
/// one-off chains like ethereum sepolia, we should make this configurable
///
/// See: https://0x.org/docs/introduction/0x-cheat-sheet#exchange-proxy-addresses
const EXCHANGE_PROXY_ADDRESS: &str = "0xdef1c0ded9bec7f1a1670819833240f027b25eff";

/// The price response
#[derive(Debug, Deserialize)]
pub struct PriceResponse {
    /// The price
    pub price: String,
}

impl ExecutionClient {
    /// Fetch a price for an asset
    pub async fn get_price(
        &self,
        buy_asset: &str,
        sell_asset: &str,
        amount: u128,
    ) -> Result<f64, ExecutionClientError> {
        let amount_str = amount.to_string();
        let params =
            [(BUY_TOKEN, buy_asset), (SELL_TOKEN, sell_asset), (SELL_AMOUNT, amount_str.as_str())];

        let resp: PriceResponse = self.send_get_request(PRICE_ENDPOINT, &params).await?;
        resp.price.parse::<f64>().map_err(ExecutionClientError::parse)
    }

    /// Fetch a quote for an asset
    pub async fn get_quote(
        &self,
        buy_asset: Address,
        sell_asset: Address,
        amount: u128,
        wallet: &LocalWallet,
    ) -> Result<ExecutionQuote, ExecutionClientError> {
        // First, set an approval for the sell token, the 0x api will not give a quote
        // if its contract is not an approved spender for the requested amount
        let exchange_addr = Address::from_str(EXCHANGE_PROXY_ADDRESS).unwrap();
        self.approve_erc20_allowance(sell_asset, exchange_addr, U256::from(amount), wallet).await?;

        let buy = format!("{buy_asset:#x}");
        let sell = format!("{sell_asset:#x}");
        let recipient = format!("{:#x}", wallet.address());
        let amount_str = amount.to_string();
        let params = [
            (BUY_TOKEN, buy.as_str()),
            (SELL_TOKEN, sell.as_str()),
            (SELL_AMOUNT, amount_str.as_str()),
            (TAKER_ADDRESS, recipient.as_str()),
        ];

        self.send_get_request(QUOTE_ENDPOINT, &params).await
    }

    /// Approve an erc20 allowance
    async fn approve_erc20_allowance(
        &self,
        token_address: Address,
        spender: Address,
        amount: U256,
        wallet: &LocalWallet,
    ) -> Result<(), ExecutionClientError> {
        let client = self.get_signer(wallet.clone());
        let erc20 = ERC20::new(token_address, Arc::new(client));

        // First, check if the allowance is already sufficient
        let allowance = erc20
            .allowance(wallet.address(), spender)
            .await
            .map_err(ExecutionClientError::arbitrum)?;
        if allowance >= amount {
            info!("Already approved erc20 allowance for {spender:#x}");
            return Ok(());
        }

        // Otherwise, approve the allowance
        let tx = erc20.approve(spender, amount);
        let pending_tx = tx.send().await.map_err(ExecutionClientError::arbitrum)?;

        let receipt = pending_tx
            .await
            .map_err(ExecutionClientError::arbitrum)?
            .ok_or_else(|| ExecutionClientError::arbitrum("Transaction failed"))?;
        info!("Approved erc20 allowance at: {:#x}", receipt.transaction_hash);
        Ok(())
    }
}
