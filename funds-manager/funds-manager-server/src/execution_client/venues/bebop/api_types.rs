//! Bebop API type definitions

#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use std::{collections::HashMap, fmt::Display};

use alloy_primitives::{Address, Bytes, U256};
use renegade_common::types::{chain::Chain, token::Token};
use serde::{Deserialize, Serialize};

use crate::execution_client::error::ExecutionClientError;

/// The subset of Bebop quote request query parameters that we support.
///
/// See: <https://api.bebop.xyz/router/ethereum/docs#/v1/get_quote_v1_quote_get>
#[derive(Serialize, Deserialize)]
pub struct BebopQuoteParams {
    /// The tokens that will be supplied by the taker.
    ///
    /// This is a comma-separated list of token addresses.
    pub sell_tokens: String,
    /// The tokens that will be supplied by the maker.
    ///
    /// This is a comma-separated list of token addresses.
    pub buy_tokens: String,
    /// The amount of each taker token, order respective to sell_tokens.
    ///
    /// This is a comma-separated list of amounts in atoms.
    pub sell_amounts: String,
    /// Address which will sign the order
    pub taker_address: String,
    /// The token approval type to use for the quoted order.
    pub approval_type: ApprovalType,
    /// Whether the solver should execute the order & fold gas costs
    /// into the price.
    ///
    /// Set to `false` to self-execute.
    pub gasless: bool,
    /// The slippage tolerance to use.
    pub slippage: f64,
    /// Whether to skip taker validation checks.
    pub skip_validation: bool,
    /// Whether to skip taker checks.
    ///
    /// The difference between this and `skip_validation` is undocumented
    /// in the Bebop docs.
    pub skip_taker_checks: bool,
    /// Referral partner that will be associated with the quote (us).
    pub source: String,
}

/// The type of approval to use for the quoted order.
///
/// We currently only support standard ERC20 approval.
#[derive(Serialize, Deserialize)]
pub enum ApprovalType {
    Standard,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopQuoteResponse {
    routes: Vec<BebopRoute>,
    best_price: BebopRouteSource,
}

impl BebopQuoteResponse {
    /// Get the winning route (JAMv2 vs PMMv3) for the quote
    pub fn best_route(&self) -> Result<&BebopRoute, ExecutionClientError> {
        self.routes
            .iter()
            .find(|route| route.route_source == BebopRouteSource::PMMv3)
            .ok_or(ExecutionClientError::custom("Winning Bebop route not found"))
    }

    /// Get the sell token for the quote
    pub fn sell_token(&self, chain: Chain) -> Result<Token, ExecutionClientError> {
        let sell_token_address = self
            .best_route()?
            .quote
            .sell_tokens
            .keys()
            .next()
            .ok_or(ExecutionClientError::custom("No sell token found"))?;

        Ok(Token::from_addr_on_chain(sell_token_address, chain))
    }

    /// Get the buy token for the quote
    pub fn buy_token(&self, chain: Chain) -> Result<Token, ExecutionClientError> {
        let buy_token_address = self
            .best_route()?
            .quote
            .buy_tokens
            .keys()
            .next()
            .ok_or(ExecutionClientError::custom("No buy token found"))?;

        Ok(Token::from_addr_on_chain(buy_token_address, chain))
    }

    /// Get the sell amount for the quote
    pub fn sell_amount(&self) -> Result<U256, ExecutionClientError> {
        let bebop_sell_token = self
            .best_route()?
            .quote
            .sell_tokens
            .values()
            .next()
            .ok_or(ExecutionClientError::custom("No sell token found"))?;

        Ok(bebop_sell_token.amount)
    }

    /// Get the buy amount for the quote
    pub fn buy_amount(&self) -> Result<U256, ExecutionClientError> {
        let bebop_buy_token = self
            .best_route()?
            .quote
            .buy_tokens
            .values()
            .next()
            .ok_or(ExecutionClientError::custom("No buy token found"))?;

        Ok(bebop_buy_token.amount)
    }

    /// Get the `to` address for the quote
    pub fn get_to_address(&self) -> Result<Address, ExecutionClientError> {
        self.best_route().map(|route| route.quote.tx.to)
    }

    /// Get the `from` address for the quote
    pub fn get_from_address(&self) -> Result<Address, ExecutionClientError> {
        self.best_route().map(|route| route.quote.tx.from)
    }

    /// Get the `value` for the quote
    pub fn get_value(&self) -> Result<U256, ExecutionClientError> {
        self.best_route().map(|route| route.quote.tx.value)
    }

    /// Get the calldata for the quote
    pub fn get_data(&self) -> Result<Bytes, ExecutionClientError> {
        self.best_route().map(|route| route.quote.tx.data.clone())
    }

    /// Get the gas limit for the quote
    pub fn get_gas_limit(&self) -> Result<U256, ExecutionClientError> {
        self.best_route().map(|route| route.quote.tx.gas)
    }

    /// Get the approval target for the quote
    pub fn get_approval_target(&self) -> Result<Address, ExecutionClientError> {
        self.best_route().map(|route| route.quote.approval_target)
    }

    /// Get the route source (JAMv2 vs PMMv3) for the quote
    pub fn get_route_source(&self) -> Result<BebopRouteSource, ExecutionClientError> {
        self.best_route().map(|route| route.route_source)
    }
}

#[derive(Deserialize, PartialEq, Eq, Hash, Debug, Clone, Copy)]
pub enum BebopRouteSource {
    JAMv2,
    PMMv3,
}

impl Display for BebopRouteSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BebopRouteSource::JAMv2 => write!(f, "JAMv2"),
            BebopRouteSource::PMMv3 => write!(f, "PMMv3"),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub struct BebopRoute {
    #[serde(rename = "type")]
    route_source: BebopRouteSource,
    quote: BebopQuote,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopQuote {
    buy_tokens: HashMap<String, BebopToken>,
    sell_tokens: HashMap<String, BebopToken>,
    approval_target: Address,
    tx: BebopTxData,
}
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopToken {
    amount: U256,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopTxData {
    from: Address,
    to: Address,
    value: U256,
    data: Bytes,
    gas: U256,
}
