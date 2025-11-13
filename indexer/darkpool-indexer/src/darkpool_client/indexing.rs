//! Event & transaction indexing logic for the darkpool contract

use std::collections::VecDeque;

use alloy::{
    hex,
    primitives::TxHash,
    providers::{Provider, ext::DebugApi},
    rpc::types::trace::geth::{
        CallFrame, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingOptions,
        GethTrace,
    },
    sol_types::SolCall,
};
use renegade_circuit_types::{balance::BalanceShare, traits::BaseType};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::depositNewBalanceCall;

use crate::{
    darkpool_client::{
        DarkpoolClient,
        error::DarkpoolClientError,
        utils::{get_selector, scalar_to_b256},
    },
    state_transitions::types::{CreateBalanceTransition, StateTransition},
};

// ---------------------------
// | Public Indexing Methods |
// ---------------------------

impl DarkpoolClient {
    /// Get the state transition associated with the registration of the given
    /// recovery ID in the given transaction
    pub async fn get_state_transition_for_recovery_id(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
    ) -> Result<StateTransition, DarkpoolClientError> {
        let recovery_id_registration_call =
            self.find_recovery_id_registration_call(recovery_id, tx_hash).await?;

        let calldata = recovery_id_registration_call.input;
        let selector = get_selector(&calldata);

        match selector {
            depositNewBalanceCall::SELECTOR => {
                self.compute_deposit_new_balance_state_transition(recovery_id, tx_hash, &calldata)
                    .await
            },
            // TODO: Implement getting intent registration transitions
            _ => Err(DarkpoolClientError::InvalidSelector(hex::encode_prefixed(selector))),
        }
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
                call.logs
                    .iter()
                    .any(|log| log.topics.clone().unwrap_or_default().contains(&recovery_id_topic))
            })
            .ok_or(DarkpoolClientError::RecoveryIdNotFound)
    }

    /// Fetch all darkpool calls made in the given transaction
    pub async fn fetch_darkpool_calls_in_tx(
        &self,
        tx_hash: TxHash,
    ) -> Result<Vec<CallFrame>, DarkpoolClientError> {
        let trace = self.fetch_call_trace(tx_hash).await?;
        Ok(self.find_darkpool_calls(&trace))
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
            ..Default::default()
        };

        self.provider()
            .debug_trace_transaction(tx_hash, options)
            .await
            .map_err(DarkpoolClientError::rpc)
    }

    /// Find all darkpool calls in a call trace
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
                darkpool_calls.push(call.clone());
            }

            // Add the sub-calls to the queue
            calls.extend(call.calls);
        }

        darkpool_calls
    }

    /// Compute the state transition associated with a `depositNewBalance` call
    async fn compute_deposit_new_balance_state_transition(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, DarkpoolClientError> {
        let registration_tx = self
            .provider()
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(DarkpoolClientError::rpc)?
            .ok_or(DarkpoolClientError::rpc(format!(
                "Transaction receipt not found for tx {tx_hash:#x}"
            )))?;

        let block_number = registration_tx
            .block_number
            .ok_or(DarkpoolClientError::rpc("Block number not found in tx {tx_hash:#x} receipt"))?;

        let deposit_new_balance_call = depositNewBalanceCall::abi_decode(calldata)
            .map_err(DarkpoolClientError::calldata_decode)?;

        let mut public_shares_iter = deposit_new_balance_call
            .newBalanceProofBundle
            .statement
            .newBalancePublicShares
            .iter()
            .map(u256_to_scalar);

        let public_share = BalanceShare::from_scalars(&mut public_shares_iter);

        Ok(StateTransition::CreateBalance(CreateBalanceTransition {
            recovery_id,
            block_number,
            public_share,
        }))
    }
}
