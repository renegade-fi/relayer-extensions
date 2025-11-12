//! Event & transaction indexing logic for the darkpool contract

use std::collections::VecDeque;

use alloy::{
    hex,
    primitives::TxHash,
    providers::ext::DebugApi,
    rpc::types::trace::geth::{
        CallFrame, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingOptions,
        GethTrace,
    },
    sol_types::SolCall,
};
use renegade_circuit_types::{Amount, balance::Balance, traits::BaseType};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    depositCall, depositNewBalanceCall, payFeesCall, settleMatchCall, withdrawCall,
};

use crate::{
    darkpool_client::{
        DarkpoolClient,
        error::DarkpoolClientError,
        utils::{get_selector, scalar_to_b256},
    },
    types::StateObjectType,
};

// ---------
// | Types |
// ---------

/// All the data necessary for indexing a state object update
pub struct StateObjectUpdate {
    /// The type of the state object that was updated
    pub state_object_type: StateObjectType,
    /// The updated public shares of the state object
    pub updated_public_shares: Vec<Scalar>,
    /// The start index of the updated public shares within the secret-sharing
    /// of the state object
    pub updated_shares_index: usize,
}

// ---------------------------
// | Public Indexing Methods |
// ---------------------------

impl DarkpoolClient {
    /// Fetch the updated public shares associated with the given spent
    /// nullifier in the given transaction
    pub async fn fetch_updated_public_shares(
        &self,
        spent_nullifier: Scalar,
        tx_hash: TxHash,
    ) -> Result<StateObjectUpdate, DarkpoolClientError> {
        let nullifying_call = self.find_nullifying_call(spent_nullifier, tx_hash).await?;

        let selector = get_selector(&nullifying_call.input);

        match selector {
            depositCall::SELECTOR => self.parse_deposit_state_object_update(&nullifying_call.input),
            depositNewBalanceCall::SELECTOR => {
                self.parse_deposit_new_balance_state_object_update(&nullifying_call.input)
            },
            withdrawCall::SELECTOR => {
                self.parse_withdraw_state_object_update(&nullifying_call.input)
            },
            payFeesCall::SELECTOR => {
                self.parse_pay_fees_state_object_update(&nullifying_call.input)
            },
            settleMatchCall::SELECTOR => todo!(),
            _ => Err(DarkpoolClientError::InvalidSelector(hex::encode_prefixed(selector))),
        }
    }

    /// Find the call that spent the given nullifier in the given transaction
    pub async fn find_nullifying_call(
        &self,
        spent_nullifier: Scalar,
        tx_hash: TxHash,
    ) -> Result<CallFrame, DarkpoolClientError> {
        let calls = self.fetch_darkpool_calls_in_tx(tx_hash).await?;
        let spent_nullifier_topic = scalar_to_b256(spent_nullifier);

        calls
            .into_iter()
            .find(|call| {
                call.logs.iter().any(|log| {
                    log.topics.clone().unwrap_or_default().contains(&spent_nullifier_topic)
                })
            })
            .ok_or(DarkpoolClientError::NullifierNotFound)
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
            .map_err(DarkpoolClientError::call_trace)
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

    /// Parse the state object update associated with a `deposit` call
    fn parse_deposit_state_object_update(
        &self,
        calldata: &[u8],
    ) -> Result<StateObjectUpdate, DarkpoolClientError> {
        let deposit_call =
            depositCall::abi_decode(calldata).map_err(DarkpoolClientError::calldata_decode)?;

        let updated_public_shares =
            vec![u256_to_scalar(&deposit_call.depositProofBundle.statement.newAmountShare)];

        // Only the amount (final field of the balance) is updated
        let updated_shares_index = Balance::NUM_SCALARS - Amount::NUM_SCALARS;

        Ok(StateObjectUpdate {
            state_object_type: StateObjectType::Balance,
            updated_public_shares,
            updated_shares_index,
        })
    }

    /// Parse the state object update associated with a `depositNewBalance` call
    fn parse_deposit_new_balance_state_object_update(
        &self,
        calldata: &[u8],
    ) -> Result<StateObjectUpdate, DarkpoolClientError> {
        let deposit_new_balance_call = depositNewBalanceCall::abi_decode(calldata)
            .map_err(DarkpoolClientError::calldata_decode)?;

        let updated_public_shares = deposit_new_balance_call
            .newBalanceProofBundle
            .statement
            .newBalancePublicShares
            .iter()
            .map(u256_to_scalar)
            .collect();

        // All of the public shares are updated
        let updated_shares_index = 0;

        Ok(StateObjectUpdate {
            state_object_type: StateObjectType::Balance,
            updated_public_shares,
            updated_shares_index,
        })
    }

    /// Parse the state object update associated with a `withdraw` call
    fn parse_withdraw_state_object_update(
        &self,
        calldata: &[u8],
    ) -> Result<StateObjectUpdate, DarkpoolClientError> {
        let withdraw_call =
            withdrawCall::abi_decode(calldata).map_err(DarkpoolClientError::calldata_decode)?;

        let updated_public_shares =
            vec![u256_to_scalar(&withdraw_call.withdrawalProofBundle.statement.newAmountShare)];

        // Only the amount (final field of the balance) is updated
        let updated_shares_index = Balance::NUM_SCALARS - Amount::NUM_SCALARS;

        Ok(StateObjectUpdate {
            state_object_type: StateObjectType::Balance,
            updated_public_shares,
            updated_shares_index,
        })
    }

    /// Parse the state object update associated with a `payFees` call
    fn parse_pay_fees_state_object_update(
        &self,
        calldata: &[u8],
    ) -> Result<StateObjectUpdate, DarkpoolClientError> {
        let pay_fees_call =
            payFeesCall::abi_decode(calldata).map_err(DarkpoolClientError::calldata_decode)?;

        let updated_public_shares = pay_fees_call
            .feePaymentProofBundle
            .statement
            .newBalancePublicShares
            .iter()
            .map(u256_to_scalar)
            .collect();

        // The relayer fee, protocol fee, and amount (final three fields of the balance)
        // are updated
        let updated_shares_index = Balance::NUM_SCALARS - Amount::NUM_SCALARS * 3;

        Ok(StateObjectUpdate {
            state_object_type: StateObjectType::Balance,
            updated_public_shares,
            updated_shares_index,
        })
    }
}
