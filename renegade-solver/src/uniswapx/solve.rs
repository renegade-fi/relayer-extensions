//! Code for solving order routes

use alloy::primitives::Address;
use alloy_primitives::U256;
use renegade_sdk::types::ExternalOrder;
use tracing::info;

use crate::{
    error::{SolverError, SolverResult},
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
        info!(
            "Found serviceable order for {} {}, mps_in: {} -> {} {}, mps_out: {}",
            input.amount,
            input.token,
            input.mpsPerPriorityFeeWei,
            order.total_output_amount(),
            order.output_token().get_alloy_address(),
            order.outputs[0].mpsPerPriorityFeeWei
        );

        info!(
            "Order first output: {}, total_output_amount: {}",
            order.outputs[0].amount,
            order.total_output_amount()
        );

        // Compute priority fee
        let priority_order_price = order.get_price()?;
        let renegade_price = self.get_renegade_price(&order).await?;
        let is_sell = order.is_sell();
        let priority_fee_wei = compute_priority_fee(priority_order_price, renegade_price, is_sell);

        // Scale the order
        let scaled_input = order.input.scale(priority_fee_wei)?;
        let scaled_output = order.total_scaled_output_amount(priority_fee_wei)?;
        info!(
            "Input scaled from {} to {}, amount scaled by {:.2}x",
            input.amount,
            scaled_input,
            u256_to_u128(scaled_input)? as f64 / u256_to_u128(input.amount)? as f64
        );
        info!(
            "Output scaled from {} to {}, amount scaled by {:.2}x",
            order.total_output_amount(),
            scaled_output,
            u256_to_u128(scaled_output)? as f64 / u256_to_u128(order.total_output_amount())? as f64
        );

        // Find a solution for the order
        let external_order = self.build_scaled_order(&order, priority_fee_wei)?;
        let renegade_bundle = self.solve_renegade_leg(external_order).await?;
        if let Some(bundle) = renegade_bundle {
            info!("Found renegade solution with output amount: {}", bundle.receive.amount);
            // Negative delta means the renegade bundle is smaller than the order
            let signed_delta = bundle.receive.amount as i128 - u256_to_u128(scaled_output)? as i128;
            if signed_delta < 0 {
                info!("Renegade bundle is smaller than the order by {}", signed_delta);
            }
        } else {
            info!("No renegade solution found");
        }

        Ok(())
    }

    /// Build an ExternalOrder from a PriorityOrder with scaled input and output
    /// amounts
    fn build_scaled_order(
        &self,
        order: &PriorityOrder,
        priority_fee: U256,
    ) -> SolverResult<ExternalOrder> {
        let scaled_input = order.input.scale(priority_fee)?;
        let scaled_output = order.total_scaled_output_amount(priority_fee)?;

        // Assert only one of {scaled_input, scaled_output} was scaled
        if scaled_input != order.input.amount && scaled_output != order.total_output_amount() {
            return Err(SolverError::InputOutputScaling);
        }

        let input_u128 = u256_to_u128(scaled_input)?;
        let order = self.build_order(
            order.input.token,
            order.output_token().get_alloy_address(),
            input_u128,
        )?;

        Ok(order)
    }

    /// A temporary (more restrictive) set of order filters while we keep the
    /// solver simple
    ///
    /// TODO: Loosen and remove this method's checks in follow-ups
    fn temporary_order_filter(&self, order: &PriorityOrder) -> SolverResult<bool> {
        // For now, we only support orders with 1 output token
        if !order.outputs.is_empty() {
            let first_output_token = order.outputs[0].token;
            for output in order.outputs.iter() {
                if output.token != first_output_token {
                    return Ok(false);
                }
            }
        }

        // For now, we only support trades that can be entirely filled by Renegade
        // This is a pair of supported tokens in which one is USDC
        let input_token = order.input.token;
        let output_token = order.output_token().get_alloy_address();
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
