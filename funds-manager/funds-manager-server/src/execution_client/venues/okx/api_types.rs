//! Okx API type definitions

#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use std::str::FromStr;

use alloy_primitives::{Address, Bytes, U256};
use renegade_common::types::{chain::Chain, token::Token};
use serde::{Deserialize, Serialize};

use crate::{
    execution_client::{error::ExecutionClientError, venues::quote::CrossVenueQuoteSource},
    helpers::from_chain_id,
};

#[derive(Deserialize)]
pub struct OkxApiResponse<T> {
    pub code: String,
    pub data: T,
}

#[derive(Deserialize)]
pub struct OkxLiquiditySource {
    pub id: String,
    pub logo: String,
    pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxApproveRequestParams {
    pub chain_id: String,
    pub token_contract_address: String,
    pub approve_amount: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxApproveResponse {
    pub dex_contract_address: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxSwapRequestParams {
    pub chain_id: String,
    pub amount: String,
    pub from_token_address: String,
    pub to_token_address: String,
    pub slippage: String,
    pub user_wallet_address: String,
    pub dex_ids: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxSwapResponse {
    pub router_result: OkxRouterResult,
    pub tx: OkxTxModel,
}

impl OkxSwapResponse {
    /// Get the chain for the quote
    pub fn chain(&self) -> Result<Chain, ExecutionClientError> {
        self.router_result
            .chain_id
            .parse()
            .map_err(ExecutionClientError::parse)
            .and_then(|chain_id| from_chain_id(chain_id).map_err(ExecutionClientError::parse))
    }

    /// Get the sell token for the quote
    pub fn sell_token(&self) -> Result<Token, ExecutionClientError> {
        let chain = self.chain()?;
        Ok(Token::from_addr_on_chain(&self.router_result.from_token.token_contract_address, chain))
    }

    /// Get the sell token address for the quote
    pub fn sell_token_address(&self) -> String {
        self.router_result.from_token.token_contract_address.clone()
    }

    /// Get the buy token for the quote
    pub fn buy_token(&self) -> Result<Token, ExecutionClientError> {
        let chain = self.chain()?;
        Ok(Token::from_addr_on_chain(&self.router_result.to_token.token_contract_address, chain))
    }

    /// Get the sell amount for the quote
    pub fn sell_amount(&self) -> U256 {
        self.router_result.from_token_amount
    }

    /// Get the buy amount for the quote
    pub fn buy_amount(&self) -> U256 {
        self.router_result.to_token_amount
    }

    /// Get the cross-venue quote source for the quote, which will include all
    /// of the liquidity sources used in the swap route
    pub fn quote_source(&self) -> CrossVenueQuoteSource {
        let dex_names = self
            .router_result
            .dex_router_list
            .iter()
            .flat_map(|router| router.sub_router_list.as_slice())
            .flat_map(|sub_router| sub_router.dex_protocol.as_slice())
            .map(|dex| dex.dex_name.clone())
            .collect();

        CrossVenueQuoteSource::Okx(dex_names)
    }

    /// Get the `to` address for the quote
    pub fn get_to_address(&self) -> Result<Address, ExecutionClientError> {
        Address::from_str(&self.tx.to).map_err(ExecutionClientError::parse)
    }

    /// Get the `from` address for the quote
    pub fn get_from_address(&self) -> Result<Address, ExecutionClientError> {
        Address::from_str(&self.tx.from).map_err(ExecutionClientError::parse)
    }

    /// Get the `value` for the quote
    pub fn get_value(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str(&self.tx.value).map_err(ExecutionClientError::parse)
    }

    /// Get the `data` for the quote
    pub fn get_data(&self) -> Result<Bytes, ExecutionClientError> {
        Bytes::from_str(&self.tx.data).map_err(ExecutionClientError::parse)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxRouterResult {
    chain_id: String,
    from_token: OkxRouterToken,
    to_token: OkxRouterToken,
    from_token_amount: U256,
    to_token_amount: U256,
    dex_router_list: Vec<OkxRouter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxRouterToken {
    token_contract_address: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxRouter {
    sub_router_list: Vec<OkxSubRouter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxSubRouter {
    dex_protocol: Vec<OkxDexProtocol>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxDexProtocol {
    dex_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxTxModel {
    from: String,
    to: String,
    value: String,
    data: String,
}
