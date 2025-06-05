//! Code for solving order routes

use tracing::info;

use crate::{
    error::SolverResult,
    uniswapx::{api_types::OrderEntity, UniswapXSolver},
};

impl UniswapXSolver {
    /// Solve a set of orders and submit solutions to the reactor
    pub(crate) async fn solve_order(&self, order: OrderEntity) -> SolverResult<()> {
        // If the order is not serviceable, print it and return
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
    ///
    /// An order is serviceable if one of the input or output tokens are
    /// supported by the Renegade API.
    ///
    /// If both tokens are supported, we can route the entire trade through the
    /// darkpool. Otherwise, we can build a two-legged trade brokered
    /// through USDC
    pub(crate) async fn is_order_serviceable(&self, order: &OrderEntity) -> bool {
        // TODO: Generalize across all output tokens
        let input_token = &order.input.token;
        let first_output_token = &order.outputs[0].token;

        let input_known = self.is_token_supported(input_token).await;
        let output_known = self.is_token_supported(first_output_token).await;

        // If either token is known, the order is serviceable
        // TODO: Exclude USDC orders paired with unsupported tokens
        // TODO: Include native ETH orders
        input_known || output_known
    }
}
