//! Defines `ExecutorClient` helpers that allow for interacting with the
//! executor contract
use alloy::rpc::types::TransactionReceipt;
use alloy_sol_types::SolCall;
use renegade_solidity_abi::IDarkpool::{
    processAtomicMatchSettleCall, MatchAtomicLinkingProofs, MatchAtomicProofs, PartyMatchPayload,
    SignedOrder, ValidMatchSettleAtomicStatement,
};

use alloy_primitives::U256;

use crate::uniswapx::abis::conversion::u256_to_u128;
use crate::uniswapx::executor_client::{errors::ExecutorError, ExecutorClient};

impl ExecutorClient {
    // -----------
    // | GETTERS |
    // -----------

    /// Parse calldata for Darkpool's processAtomicMatchSettle
    fn parse_calldata_for_execute_atomic_match_settle(
        &self,
        calldata: &[u8],
    ) -> Result<
        (
            PartyMatchPayload,
            ValidMatchSettleAtomicStatement,
            MatchAtomicProofs,
            MatchAtomicLinkingProofs,
        ),
        ExecutorError,
    > {
        let calldata = processAtomicMatchSettleCall::abi_decode(calldata)?;
        let internal_party_payload = calldata.internalPartyPayload;
        let match_settle_statement = calldata.matchSettleStatement;
        let proofs = calldata.proofs;
        let linking_proofs = calldata.linkingProofs;

        Ok((internal_party_payload, match_settle_statement, proofs, linking_proofs))
    }

    // -----------
    // | SETTERS |
    // -----------

    /// Executes a UniswapX order with atomic match settlement
    pub async fn execute_atomic_match_settle(
        &self,
        calldata: &[u8],
        signed_order: SignedOrder,
        priority_fee_wei: U256,
        auction_start_block: U256,
    ) -> Result<TransactionReceipt, ExecutorError> {
        // Wait for auction start block before submission
        let target_block = u256_to_u128(auction_start_block)? as u64;
        self.wait_for_block(target_block).await?;

        // Parse the calldata for the executeAtomicMatchSettle call
        let (internal_party_payload, match_settle_statement, proofs, linking_proofs) =
            self.parse_calldata_for_execute_atomic_match_settle(calldata)?;

        // Build the call to the executeAtomicMatchSettle function
        let call = self.contract.executeAtomicMatchSettle(
            signed_order,
            internal_party_payload,
            match_settle_statement,
            proofs,
            linking_proofs,
        );

        // Submit on-chain
        let receipt = self.send_tx(call, priority_fee_wei).await?;

        Ok(receipt)
    }
}
