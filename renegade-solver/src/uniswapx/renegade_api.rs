//! Renegade external match API helpers
#![allow(deprecated)]
use renegade_common::types::token::Token;
use renegade_sdk::types::{AtomicMatchApiBundle, ExternalOrder, OrderSide};
use renegade_sdk::ExternalMatchOptions;

use crate::{error::SolverResult, uniswapx::UniswapXSolver};

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

    /// Get the price of a bundle
    pub(crate) fn get_bundle_price(&self, bundle: &AtomicMatchApiBundle) -> SolverResult<f64> {
        let quote_amt = match bundle.match_result.direction {
            OrderSide::Buy => bundle.send.amount,
            OrderSide::Sell => bundle.receive.amount,
        };
        let base_amt = match bundle.match_result.direction {
            OrderSide::Buy => bundle.receive.amount,
            OrderSide::Sell => bundle.send.amount,
        };
        let quote_token = Token::from_addr(&bundle.match_result.quote_mint);
        let base_token = Token::from_addr(&bundle.match_result.base_mint);
        let quote_amt = quote_token.convert_to_decimal(quote_amt);
        let base_amt = base_token.convert_to_decimal(base_amt);

        Ok(quote_amt / base_amt)
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
}
