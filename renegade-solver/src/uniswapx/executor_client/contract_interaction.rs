//! Defines `ExecutorClient` helpers that allow for interacting with the
//! executor contract
use crate::uniswapx::{
    abis::conversion::u256_to_u128,
    executor_client::{errors::ExecutorError, ExecutorClient},
};
use alloy::rpc::types::TransactionRequest;
use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use renegade_sdk::types::AtomicMatchApiBundle;
use renegade_solidity_abi::IDarkpool::{sponsorAtomicMatchSettleCall, SignedOrder};

/// Gas limit for the `executeAtomicMatchSettle` function
const ATOMIC_MATCH_SETTLE_GAS_LIMIT: u64 = 10_000_000;

impl ExecutorClient {
    /// Build a TransactionRequest to submit a fill to the executor contract
    pub fn build_atomic_match_settle_tx_request(
        &self,
        bundle: AtomicMatchApiBundle,
        signed_order: SignedOrder,
        priority_fee_wei: U256,
    ) -> Result<TransactionRequest, ExecutorError> {
        let darkpool_calldata =
            bundle.settlement_tx.input.data.expect("No calldata found in bundle transaction");

        let sponsorAtomicMatchSettleCall {
            internalPartyMatchPayload: internal_party_payload,
            validMatchSettleAtomicStatement: match_settle_statement,
            matchProofs: proofs,
            matchLinkingProofs: linking_proofs,
            ..
        } = sponsorAtomicMatchSettleCall::abi_decode(darkpool_calldata.as_ref())?;

        let call = self.contract.executeAtomicMatchSettle(
            signed_order,
            internal_party_payload,
            match_settle_statement,
            proofs,
            linking_proofs,
        );

        let mut tx = call.into_transaction_request();
        tx.max_priority_fee_per_gas = Some(u256_to_u128(priority_fee_wei)?);
        tx.gas = Some(ATOMIC_MATCH_SETTLE_GAS_LIMIT);

        Ok(tx)
    }
}
