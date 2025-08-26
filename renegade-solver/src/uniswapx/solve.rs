//! Code for solving order routes

use alloy::primitives::Address;
use alloy_primitives::U256;
use renegade_sdk::types::AtomicMatchApiBundle;
use tracing::info;

use crate::planner::compute_send_plan;
use crate::planner::Measurements;
use crate::tx_store::store::L2Position;
use crate::tx_store::store::TxTiming;
use crate::uniswapx::abis::conversion::u256_to_u128;
use crate::{
    error::SolverResult,
    uniswapx::{
        abis::uniswapx::PriorityOrderReactor::PriorityOrder, priority_fee::compute_priority_fee,
        uniswap_api::types::OrderEntity, UniswapXSolver,
    },
};

impl UniswapXSolver {
    /// Solve a set of orders and write solution calldata to the TxStore
    pub(crate) async fn solve_order(&self, api_order: OrderEntity) -> SolverResult<()> {
        // Decode the ABI encoded order
        let order = api_order.decode_priority_order()?;

        // Check if the order is serviceable
        if !self.is_order_serviceable(&order)? || !self.temporary_order_filter(&order)? {
            return Ok(());
        }

        self.log_serviceable_order_summary(&api_order)?;

        // Find a solution for the order
        let external_order = order.to_external_order()?;
        let renegade_bundle = self.solve_renegade_leg(external_order).await?;
        if let Some(bundle) = renegade_bundle {
            self.log_renegade_solution_found(&bundle);

            // Compute priority fee
            let uniswapx_price = order.get_price()?;
            let renegade_price = self.get_bundle_price(&bundle)?;
            let is_sell = order.is_sell();
            let mut priority_fee_wei =
                compute_priority_fee(uniswapx_price, renegade_price, is_sell);

            self.log_computed_priority_fee(priority_fee_wei);

            // Add baseline priority fee
            priority_fee_wei = priority_fee_wei.saturating_add(order.baselinePriorityFeeWei);

            // Write to TxStore
            self.write_tx_record(&api_order, bundle, priority_fee_wei)?;
        } else {
            info!("No renegade solution found");
        }

        Ok(())
    }

    /// Write a tx record to the TxStore
    fn write_tx_record(
        &self,
        api_order: &OrderEntity,
        bundle: AtomicMatchApiBundle,
        priority_fee_wei: U256,
    ) -> SolverResult<()> {
        let order = api_order.decode_priority_order()?;
        let signed_order = api_order.decode_signed_order()?;
        let order_hash = api_order.order_hash.clone();
        let auction_start_block = order.auction_start_block();

        let tx = self.executor_client.build_atomic_match_settle_tx_request(
            bundle,
            signed_order,
            priority_fee_wei,
        )?;

        let start_block = u256_to_u128(auction_start_block)? as u64;
        let target = L2Position { l2_block: start_block, flashblock: 1 };

        let plan = compute_send_plan(target, &Measurements::default());
        let timing: TxTiming = plan.into();
        self.tx_store.enqueue_with_timing(&order_hash, tx, timing.clone())?;

        self.log_writing_tx_record(&order_hash, priority_fee_wei, &timing);

        Ok(())
    }

    /// A temporary (more restrictive) set of order filters while we keep the
    /// solver simple
    ///
    /// TODO: Loosen and remove this method's checks in follow-ups
    fn temporary_order_filter(&self, order: &PriorityOrder) -> SolverResult<bool> {
        // For now, we only support orders with the same token for all outputs
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

impl UniswapXSolver {
    /// Log a concise summary of a serviceable order
    fn log_serviceable_order_summary(&self, api_order: &OrderEntity) -> SolverResult<()> {
        let order = api_order.decode_priority_order()?;
        let order_hash = api_order.order_hash.clone();
        let auction_start_block = order.auction_start_block();

        let input = &order.input;
        info!(
            "Serviceable order: input {} {} (mps_in: {}), output {} {} (mps_out: {}), hash: {}, auction_start_block: {}",
            input.amount,
            input.token,
            input.mpsPerPriorityFeeWei,
            order.total_output_amount(),
            order.output_token().get_alloy_address(),
            order.outputs[0].mpsPerPriorityFeeWei,
            order_hash,
            auction_start_block
        );
        Ok(())
    }

    /// Log details when a Renegade solution is found
    fn log_renegade_solution_found(&self, bundle: &AtomicMatchApiBundle) {
        info!(
            "Found renegade solution with input amount: {}, output amount: {}",
            bundle.send.amount, bundle.receive.amount
        );
    }

    /// Log the computed priority fee in wei and ETH
    fn log_computed_priority_fee(&self, priority_fee_wei: U256) {
        info!(
            "Computed priority fee: {} wei ({} ETH)",
            priority_fee_wei,
            priority_fee_wei.to_string().parse::<f64>().unwrap_or(0.0) / 1e18
        );
    }

    /// Log that we are writing the tx record along with trigger metadata
    fn log_writing_tx_record(&self, order_hash: &str, priority_fee_wei: U256, timing: &TxTiming) {
        info!(
            trigger_l2 = ?timing.trigger.l2_block,
            trigger_flashblock = ?timing.trigger.flashblock,
            order_hash = ?order_hash,
            priority_fee_wei = ?priority_fee_wei,
            "Writing tx record"
        );
    }
}
