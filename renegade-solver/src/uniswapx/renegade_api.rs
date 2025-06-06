//! Renegade external match API helpers
#![allow(deprecated)]

use alloy::primitives::Address;
use renegade_sdk::{
    types::{AtomicMatchApiBundle, ExternalOrder, OrderSide},
    ExternalMatchOptions, ExternalOrderBuilder,
};

use crate::{error::SolverResult, uniswapx::UniswapXSolver};

impl UniswapXSolver {
    /// Fetch a match bundle for one leg of a trade
    ///
    /// Assumes that one of the tokens is USDC and the other is supported by the
    /// Renegade API. Validation for this should be done in the caller.
    pub(crate) async fn solve_renegade_leg(
        &self,
        input_token: Address,
        output_token: Address,
        input_amount: u128,
    ) -> SolverResult<Option<AtomicMatchApiBundle>> {
        let opt = Self::get_external_match_options();
        let order = self.build_order(input_token, output_token, input_amount)?;
        let maybe_bundle =
            self.renegade_client.request_external_match_with_options(order, opt).await?;

        Ok(maybe_bundle.map(|b| b.match_bundle))
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

    /// Build an order for the given token pair
    fn build_order(
        &self,
        input_token: Address,
        output_token: Address,
        input_amount: u128,
    ) -> SolverResult<ExternalOrder> {
        let is_buy_side = self.is_usdc(input_token);
        if is_buy_side {
            // Base is output token, quote is input token
            self.build_buy_order(output_token, input_token, input_amount)
        } else {
            // Base is input token, quote is output token
            self.build_sell_order(input_token, output_token, input_amount)
        }
    }

    /// Build a buy order for the given token pair
    fn build_buy_order(
        &self,
        base: Address,
        quote: Address,
        in_amount: u128,
    ) -> SolverResult<ExternalOrder> {
        let order = ExternalOrderBuilder::new()
            .base_mint(&base.to_string())
            .quote_mint(&quote.to_string())
            .quote_amount(in_amount)
            .side(OrderSide::Buy)
            .build()?;
        Ok(order)
    }

    /// Build a sell order for the given token pair
    fn build_sell_order(
        &self,
        base: Address,
        quote: Address,
        in_amount: u128,
    ) -> SolverResult<ExternalOrder> {
        let order = ExternalOrderBuilder::new()
            .base_mint(&base.to_string())
            .quote_mint(&quote.to_string())
            .base_amount(in_amount)
            .side(OrderSide::Sell)
            .build()?;
        Ok(order)
    }
}
