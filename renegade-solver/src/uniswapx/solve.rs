//! Code for solving order routes

use alloy::primitives::Address;
use tracing::info;

use crate::{
    error::SolverResult,
    uniswapx::{
        abis::{conversion::u256_to_u128, uniswapx::PriorityOrderReactor::PriorityOrder},
        priority_fee::compute_priority_fee,
        uniswap_api::types::OrderEntity,
        UniswapXSolver,
    },
};

impl UniswapXSolver {
    /// Solve a set of orders and submit solutions to the reactor
    pub(crate) async fn solve_order(&self, api_order: OrderEntity) -> SolverResult<()> {
        // Decode the ABI encoded order
        // The order amounts in the raw API response are currently incorrect, so we need
        // to pull them from the ABI encoded order
        let order = api_order.decode_priority_order()?;

        // Check if the order is serviceable
        if !self.is_order_serviceable(&order)? || !self.temporary_order_filter(&order)? {
            return Ok(());
        }

        // Print order details if it's serviceable
        let input = &order.input;
        let first_output = &order.outputs[0];
        info!(
            "Found serviceable order for {} {} -> {} {}",
            input.amount, input.token, first_output.amount, first_output.token
        );

        // Compute priority fee
        let priority_order_price = self.get_priority_order_price(&order).await?;
        let renegade_price = self.get_renegade_price(&order).await?;
        let is_sell = self.is_usdc(order.outputs[0].token);
        let priority_fee = compute_priority_fee(priority_order_price, renegade_price, is_sell);
        info!("Priority fee (wei): {}", priority_fee);

        // Find a solution for the order
        let in_token = order.input.token;
        let out_token = order.outputs[0].token;
        let amount = u256_to_u128(order.input.amount)?;
        let renegade_bundle = self.solve_renegade_leg(in_token, out_token, amount).await?;
        if let Some(bundle) = renegade_bundle {
            info!("Found renegade solution with receive amount: {}", bundle.receive.amount);
        } else {
            info!("No renegade solution found");
        }

        Ok(())
    }

    /// A temporary (more restrictive) set of order filters while we keep the
    /// solver simple
    ///
    /// TODO: Loosen and remove this method's checks in follow-ups
    fn temporary_order_filter(&self, order: &PriorityOrder) -> SolverResult<bool> {
        // For now we only support single-leg routes
        if order.outputs.len() != 1 {
            return Ok(false);
        }

        // For now, we only support trades that can be entirely filled by Renegade
        // This is a pair of supported tokens in which one is USDC
        let input_token = order.input.token;
        let output_token = order.outputs[0].token;
        let is_input_usdc = self.is_usdc(input_token);
        let is_output_usdc = self.is_usdc(output_token);
        let input_supported = self.is_token_supported(input_token);
        let output_supported = self.is_token_supported(output_token);

        let is_one_usdc = is_input_usdc || is_output_usdc;
        let both_supported = input_supported && output_supported;
        Ok(is_one_usdc && both_supported)
    }

    /// Decide whether an order is serviceable by the solver
    fn is_order_serviceable(&self, order: &PriorityOrder) -> SolverResult<bool> {
        let input_token = order.input.token;
        for output in order.outputs.iter() {
            if self.is_pair_serviceable(input_token, output.token)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    // -----------
    // | Helpers |
    // -----------

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
    fn is_pair_serviceable(
        &self,
        input_token: Address,
        output_token: Address,
    ) -> SolverResult<bool> {
        // At least one of the input or output token must be supported and not USDC
        let input_usdc = self.is_usdc(input_token);
        let output_usdc = self.is_usdc(output_token);
        let input_known_not_usdc = self.is_token_supported(input_token) && !input_usdc;
        let output_known_not_usdc = self.is_token_supported(output_token) && !output_usdc;
        let serviceable = input_known_not_usdc || output_known_not_usdc;
        Ok(serviceable)
    }
}
