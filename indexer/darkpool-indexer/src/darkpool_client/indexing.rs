//! Event & transaction indexing logic for the darkpool contract

use std::collections::VecDeque;

use alloy::{
    eips::BlockId,
    primitives::{Address, B256, TxHash, U256},
    providers::{Provider, ext::DebugApi},
    rpc::types::trace::geth::{
        CallConfig, CallFrame, CallLogFrame, GethDebugBuiltInTracerType, GethDebugTracerType,
        GethDebugTracingOptions, GethTrace,
    },
    sol_types::SolEvent,
};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_constants::Scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    NullifierSpent, PublicIntentUpdated, RecoveryIdRegistered,
};

use crate::darkpool_client::{DarkpoolClient, error::DarkpoolClientError, utils::scalar_to_b256};

// ---------------------------
// | Public Indexing Methods |
// ---------------------------

impl DarkpoolClient {
    /// Find the call that spent the given nullifier in the given
    /// transaction
    pub async fn find_nullifying_call(
        &self,
        nullifier: U256,
        tx_hash: TxHash,
    ) -> Result<CallFrame, DarkpoolClientError> {
        let calls = self.fetch_darkpool_calls_in_tx(tx_hash).await?;

        calls
            .into_iter()
            .find(|call| {
                call.logs.iter().any(|log| {
                    let topics = log.topics.clone().unwrap_or_default();
                    if topics.first() == Some(&NullifierSpent::SIGNATURE_HASH)
                        && let Some(data) = &log.data
                    {
                        // The nullifier is not an indexed field, so we need to ABI-decode it from
                        // the log data
                        return NullifierSpent::abi_decode_data(data)
                            .map(|(log_nullifier,)| log_nullifier == nullifier)
                            .unwrap_or(false);
                    }

                    false
                })
            })
            .ok_or(DarkpoolClientError::NullifierNotFound)
    }

    /// Find the call that registered the given recovery ID in the given
    /// transaction
    pub async fn find_recovery_id_registration_call(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
    ) -> Result<CallFrame, DarkpoolClientError> {
        let calls = self.fetch_darkpool_calls_in_tx(tx_hash).await?;
        let recovery_id_topic = scalar_to_b256(recovery_id);

        calls
            .into_iter()
            .find(|call| {
                call.logs.iter().any(|log| {
                    let topics = log.topics.clone().unwrap_or_default();
                    topics.first() == Some(&RecoveryIdRegistered::SIGNATURE_HASH)
                        && topics.contains(&recovery_id_topic)
                })
            })
            .ok_or(DarkpoolClientError::RecoveryIdNotFound)
    }

    /// Find the call that updated the given public intent in the given
    /// transaction
    pub async fn find_public_intent_update_call(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
    ) -> Result<CallFrame, DarkpoolClientError> {
        let calls = self.fetch_darkpool_calls_in_tx(tx_hash).await?;

        calls
            .into_iter()
            .find(|call| {
                call.logs.iter().any(|log| {
                    let topics = log.topics.clone().unwrap_or_default();
                    topics.first() == Some(&PublicIntentUpdated::SIGNATURE_HASH)
                        && topics.contains(&intent_hash)
                })
            })
            .ok_or(DarkpoolClientError::PublicIntentHashNotFound)
    }

    /// Fetch all darkpool calls made in the given transaction
    pub async fn fetch_darkpool_calls_in_tx(
        &self,
        tx_hash: TxHash,
    ) -> Result<Vec<CallFrame>, DarkpoolClientError> {
        let trace = self.fetch_call_trace(tx_hash).await?;
        Ok(self.find_darkpool_calls(&trace))
    }

    /// Get the block number of the given transaction
    pub async fn get_tx_block_number(&self, tx_hash: TxHash) -> Result<u64, DarkpoolClientError> {
        let receipt = self
            .provider()
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(DarkpoolClientError::rpc)?
            .ok_or(DarkpoolClientError::rpc(format!(
                "Transaction receipt not found for tx {tx_hash:#x}"
            )))?;

        let block_number = receipt.block_number.ok_or(DarkpoolClientError::rpc(format!(
            "Block number not found in tx {tx_hash:#x} receipt"
        )))?;

        Ok(block_number)
    }

    /// Get the protocol fee rate for the given pair at the given block number
    pub async fn get_protocol_fee_rate_at_block(
        &self,
        asset0: Address,
        asset1: Address,
        block_number: u64,
    ) -> Result<FixedPoint, DarkpoolClientError> {
        let protocol_fee_rate = self
            .darkpool
            .getProtocolFee(asset0, asset1)
            .block(BlockId::number(block_number))
            .call()
            .await
            .map_err(DarkpoolClientError::rpc)?;

        Ok(protocol_fee_rate.into())
    }
}

// ----------------------------
// | Private Indexing Helpers |
// ----------------------------

impl DarkpoolClient {
    /// Fetch the call trace for the given transaction
    async fn fetch_call_trace(&self, tx_hash: TxHash) -> Result<GethTrace, DarkpoolClientError> {
        let options = GethDebugTracingOptions {
            tracer: Some(GethDebugTracerType::BuiltInTracer(
                GethDebugBuiltInTracerType::CallTracer,
            )),
            tracer_config: CallConfig::default().with_log().into(),
            ..Default::default()
        };

        self.provider()
            .debug_trace_transaction(tx_hash, options)
            .await
            .map_err(DarkpoolClientError::rpc)
    }

    /// Find all darkpool calls in a call trace
    ///
    /// The darkpool contract is a proxy, so the top-level CALL to the
    /// darkpool address will DELEGATECALL into implementation contracts.
    /// Logs emitted via DELEGATECALL are attributed to the proxy address
    /// but attached to the DELEGATECALL sub-frame in the trace, not the
    /// parent frame. We collect all logs from each darkpool call's subtree
    /// to account for this.
    fn find_darkpool_calls(&self, trace: &GethTrace) -> Vec<CallFrame> {
        let darkpool = self.darkpool_address();
        let global_call_frame = match trace {
            GethTrace::CallTracer(frame) => frame.clone(),
            _ => return vec![],
        };

        // BFS the call tree to find all calls to the darkpool contract
        let mut darkpool_calls = vec![];
        let mut calls = VecDeque::from([global_call_frame]);
        while let Some(call) = calls.pop_front() {
            if let Some(to) = call.to
                && to == darkpool
            {
                let mut hydrated = call.clone();
                hydrated.logs = collect_subtree_logs(&call, darkpool);
                darkpool_calls.push(hydrated);
            }

            // Add the sub-calls to the queue
            calls.extend(call.calls);
        }

        darkpool_calls
    }
}

/// Recursively collect all logs emitted by `address` from a call frame
/// and its sub-calls
fn collect_subtree_logs(frame: &CallFrame, address: Address) -> Vec<CallLogFrame> {
    let mut logs: Vec<CallLogFrame> =
        frame.logs.iter().filter(|log| log.address == Some(address)).cloned().collect();

    for sub_call in &frame.calls {
        logs.extend(collect_subtree_logs(sub_call, address));
    }
    logs
}
