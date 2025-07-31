//! Renegade external match API helpers
#![allow(deprecated)]

use alloy::primitives::Address;
use renegade_common::types::token::Token;
use renegade_sdk::{
    types::{AtomicMatchApiBundle, ExternalOrder, OrderSide},
    ExternalMatchOptions, ExternalOrderBuilder,
};

use crate::{
    error::SolverResult,
    uniswapx::{
        abis::uniswapx::PriorityOrderReactor::PriorityOrder, UniswapXSolver, NATIVE_ETH_ADDRESS,
        NATIVE_ETH_ADDRESS_RENEGADE, WETH_TICKER,
    },
};

impl UniswapXSolver {
    /// Fetch a match bundle for one leg of a trade
    ///
    /// Assumes that one of the tokens is USDC and the other is supported by the
    /// Renegade API. Validation for this should be done in the caller.
    pub(crate) async fn solve_renegade_leg(
        &self,
        order: ExternalOrder,
    ) -> SolverResult<Option<AtomicMatchApiBundle>> {
        let opt = Self::get_external_match_options();
        let maybe_bundle =
            self.renegade_client.request_external_match_with_options(order, opt).await?;

        Ok(maybe_bundle.map(|b| b.match_bundle))
    }

    /// Get the price of a token from the price reporter client
    ///
    /// Assumes one side of the order is USDC and there is only one output token
    pub(crate) async fn get_renegade_price(&self, order: &PriorityOrder) -> SolverResult<f64> {
        let mint = match order.base_token().get_addr().as_str() {
            NATIVE_ETH_ADDRESS => Token::from_ticker(WETH_TICKER).get_addr(),
            _ => order.base_token().get_addr(),
        };

        let price = self.price_reporter_client.get_price(&mint, self.chain_id).await?;
        Ok(price)
    }

    // -----------
    // | Helpers |
    // -----------

    /// Get external match options for a request
    ///
    /// Note that we don't explicitly request gas sponsorship here, so it will
    /// be disabled
    fn get_external_match_options() -> ExternalMatchOptions {
        ExternalMatchOptions::new().with_gas_estimation(false)
    }

    /// Map Native ETH to Renegade native ETH address
    fn map_native_eth_to_renegade_eth(&self, token: String) -> String {
        match token.as_str() {
            NATIVE_ETH_ADDRESS => NATIVE_ETH_ADDRESS_RENEGADE.to_string(),
            _ => token,
        }
    }

    /// Build an order for the given token pair
    pub(crate) fn build_order(
        &self,
        input_token: Address,
        output_token: Address,
        output_amount: u128,
    ) -> SolverResult<ExternalOrder> {
        let is_buy_side = self.is_usdc(input_token);
        if is_buy_side {
            // Base is output token, quote is input token
            self.build_buy_order(output_token, input_token, output_amount)
        } else {
            // Base is input token, quote is output token
            self.build_sell_order(input_token, output_token, output_amount)
        }
    }

    /// Build a buy order for the given token pair
    fn build_buy_order(
        &self,
        base: Address,
        quote: Address,
        output_amount: u128,
    ) -> SolverResult<ExternalOrder> {
        let base = self.map_native_eth_to_renegade_eth(base.to_string());
        let order = ExternalOrderBuilder::new()
            .base_mint(&base)
            .quote_mint(&quote.to_string())
            .exact_base_output(output_amount)
            .side(OrderSide::Buy)
            .build()?;
        Ok(order)
    }

    /// Build a sell order for the given token pair
    fn build_sell_order(
        &self,
        base: Address,
        quote: Address,
        output_amount: u128,
    ) -> SolverResult<ExternalOrder> {
        let base = self.map_native_eth_to_renegade_eth(base.to_string());
        let order = ExternalOrderBuilder::new()
            .base_mint(&base)
            .quote_mint(&quote.to_string())
            .exact_quote_output(output_amount)
            .side(OrderSide::Sell)
            .build()?;
        Ok(order)
    }
}
