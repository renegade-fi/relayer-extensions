//! Bebop API type definitions

#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
}

/// The type of approval to use for the quoted order.
///
/// We currently only support standard ERC20 approval.
#[derive(Serialize, Deserialize)]
pub enum ApprovalType {
    Standard,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopQuoteResponse {
    routes: Vec<BebopRoute>,
    best_price: BebopRouteSource,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug)]
pub enum BebopRouteSource {
    JAMv2,
    PMMv3,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", content = "quote")]
pub enum BebopRoute {
    JAMv2(BebopJamQuote),
    PMMv3(BebopPmmQuote),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopJamQuote {
    slippage: f64,
    buy_tokens: HashMap<String, BebopBuyToken>,
    sell_tokens: HashMap<String, BebopSellToken>,
    settlement_address: String,
    approval_target: String,
    price_impact: f64,
    tx: BebopTxData,
    solver: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopPmmQuote {
    slippage: f64,
    buy_tokens: HashMap<String, BebopBuyToken>,
    sell_tokens: HashMap<String, BebopSellToken>,
    settlement_address: String,
    approval_target: String,
    price_impact: f64,
    tx: BebopTxData,
    makers: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopBuyToken {
    amount: String,
    decimals: u8,
    price_usd: f64,
    symbol: String,
    price: f64,
    price_before_fee: f64,
    minimum_amount: String,
    amount_before_fee: String,
    delta_from_expected: f64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopSellToken {
    amount: String,
    decimals: u8,
    price_usd: f64,
    symbol: String,
    price: f64,
    price_before_fee: f64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BebopTxData {
    from: String,
    to: String,
    value: String,
    data: String,
}
