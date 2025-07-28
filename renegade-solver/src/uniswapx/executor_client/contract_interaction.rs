//! Defines `ExecutorClient` helpers that allow for interacting with the
//! executor contract
use alloy::rpc::types::TransactionReceipt;
use renegade_solidity_abi::IDarkpool::{
    MatchAtomicLinkingProofs, MatchAtomicProofs, PartyMatchPayload, SignedOrder,
    ValidMatchSettleAtomicStatement,
};

use crate::uniswapx::executor_client::{errors::ExecutorError, ExecutorClient};

impl ExecutorClient {
    // -----------
    // | SETTERS |
    // -----------

    /// Executes a UniswapX order with atomic match settlement
    pub async fn execute_atomic_match_settle(
        &self,
        order: SignedOrder,
        internal_party_payload: PartyMatchPayload,
        match_settle_statement: ValidMatchSettleAtomicStatement,
        proofs: MatchAtomicProofs,
        linking_proofs: MatchAtomicLinkingProofs,
    ) -> Result<TransactionReceipt, ExecutorError> {
        let call = self.contract.executeAtomicMatchSettle(
            order,
            internal_party_payload,
            match_settle_statement,
            proofs,
            linking_proofs,
        );

        let receipt = self.send_tx(call).await?;

        Ok(receipt)
    }
}
