//! Code for solving order routes

use alloy::primitives::Address;
use alloy_primitives::U256;
use alloy_rpc_types_eth::TransactionRequest;
use renegade_sdk::types::AtomicMatchApiBundle;
use tracing::info;

use crate::tx_store::store::TxTiming;
use crate::uniswapx::abis::conversion::u256_to_u128;
use crate::{
    error::{SolverError, SolverResult},
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
            let priority_fee_wei = compute_priority_fee(uniswapx_price, renegade_price, is_sell);

            // Enqueue the transaction for submission
            self.enqueue(&api_order, bundle, priority_fee_wei)?;
        } else {
            info!("No renegade solution found");
        }

        Ok(())
    }

    /// Enqueue a transaction for submission
    fn enqueue(
        &self,
        api_order: &OrderEntity,
        bundle: AtomicMatchApiBundle,
        priority_fee_wei: U256,
    ) -> SolverResult<()> {
        let order = api_order.decode_priority_order()?;
        let signed_order = api_order.decode_signed_order()?;
        let order_hash = api_order.order_hash.clone();
        let auction_start_block = order.auction_start_block();

        // Build the transaction request template
        let mut tx = self.executor_client.build_atomic_match_settle_tx_request(
            bundle,
            signed_order,
            priority_fee_wei,
        )?;

        // Add baseline priority fee
        let priority_fee_wei = priority_fee_wei.saturating_add(order.baselinePriorityFeeWei);
        // Set the transaction's latest base fee and nonce
        self.set_tx_fee(&mut tx, priority_fee_wei)?;

        // Sign the transaction and get hash
        let (raw_tx_bytes, tx_hash) = self.executor_client.sign_transaction(tx)?;

        // Compute timing
        let start_block = u256_to_u128(auction_start_block)? as u64;
        let target_timestamp_ms = self.flashblock_clock.target_timestamp_ms(1, start_block);
        let send_timestamp_ms = self.controller.compute_send_ms(target_timestamp_ms);

        // Enqueue the transaction for submission at the given timestamp
        self.tx_driver.enqueue(send_timestamp_ms, &raw_tx_bytes, &tx_hash);

        // Write the transaction context to the TxStore
        let timing = TxTiming { send_timestamp_ms, target_timestamp_ms };
        self.tx_store.write(&order_hash, &tx_hash, &timing);

        tracing::info!(
            message = "tx enqueued",
            id = order_hash,
            send_timestamp_ms = send_timestamp_ms,
            start_block = start_block,
            target_timestamp_ms = target_timestamp_ms,
            tx_hash = tx_hash.to_string(),
        );

        Ok(())
    }

    /// Set the transaction's latest base fee and nonce from the cache
    /// the cache
    fn set_tx_fee(&self, tx: &mut TransactionRequest, priority_fee_wei: U256) -> SolverResult<()> {
        let base_fee = self
            .chain_state_cache
            .base_fee_per_gas()
            .ok_or_else(|| SolverError::Custom("base_fee_per_gas unavailable".to_string()))?
            as u128;
        let nonce = self
            .chain_state_cache
            .pending_nonce()
            .ok_or_else(|| SolverError::Custom("pending nonce unavailable".to_string()))?;

        // Add 20% buffer to base fee
        let buffed_base_fee = base_fee.saturating_mul(12) / 10;
        let priority_fee_u128 = u256_to_u128(priority_fee_wei)?;
        let max_fee = buffed_base_fee.saturating_add(priority_fee_u128);

        // Complete the transaction
        tx.max_fee_per_gas = Some(max_fee);
        tx.nonce = Some(nonce);
        tx.chain_id = Some(self.executor_client.chain_id);

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
}
