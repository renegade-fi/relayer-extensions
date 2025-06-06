//! Code for solving order routes

use std::str::FromStr;

use alloy::primitives::Address;
use tracing::info;

use crate::{
    error::SolverResult,
    uniswapx::{api_types::OrderEntity, UniswapXSolver, USDC_SYMBOL},
};

impl UniswapXSolver {
    /// Solve a set of orders and submit solutions to the reactor
    pub(crate) async fn solve_order(&self, order: OrderEntity) -> SolverResult<()> {
        // Check if the order is serviceable
        if !self.is_order_serviceable(&order).await {
            return Ok(());
        }

        // Print order details if it's serviceable
        let input = &order.input;
        let first_output = &order.outputs[0];
        info!(
            "Found serviceable order for {} {} -> {} {}",
            input.amount, input.token, first_output.amount, first_output.token
        );

        // TODO: Find solutions
        Ok(())
    }

    /// Decide whether an order is serviceable by the solver
    async fn is_order_serviceable(&self, order: &OrderEntity) -> bool {
        let input_token = &order.input.token;
        for output in order.outputs.iter() {
            if self.is_pair_serviceable(input_token, &output.token).await {
                return true;
            }
        }

        false
    }

    /// Returns whether a pair is serviceable
    ///
    /// An order is serviceable if one of the input or output tokens are
    /// supported by the Renegade API.
    ///
    /// If both tokens are supported, we can route the entire trade through the
    /// darkpool. Otherwise, we can build a two-legged trade brokered
    /// through USDC
    ///
    /// Note that if the only known token is USDC, the pair is not serviceable.
    async fn is_pair_serviceable(&self, input_token: &str, output_token: &str) -> bool {
        // Parse the tokens, return false if they are not valid addresses
        let input_token = match Address::from_str(input_token) {
            Ok(addr) => addr,
            Err(_) => return false,
        };
        let output_token = match Address::from_str(output_token) {
            Ok(addr) => addr,
            Err(_) => return false,
        };

        // At least one of the input or output token must be supported and not USDC
        let input_usdc = self.is_usdc(input_token);
        let output_usdc = self.is_usdc(output_token);
        let input_known_not_usdc = self.is_token_supported(input_token) && !input_usdc;
        let output_known_not_usdc = self.is_token_supported(output_token) && !output_usdc;
        input_known_not_usdc || output_known_not_usdc
    }

    /// Returns whether the given token is USDC
    fn is_usdc(&self, token: Address) -> bool {
        let usdc_addr = self.get_token_address(USDC_SYMBOL).expect("No USDC address");
        token == usdc_addr
    }
}
