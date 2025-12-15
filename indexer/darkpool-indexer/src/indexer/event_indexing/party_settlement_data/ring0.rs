//! Utilities for constructing & interacting with ring 0 settlement data

use alloy::{
    primitives::{B256, TxHash, keccak256},
    sol_types::SolValue,
};
use renegade_circuit_types::{Amount, fixed_point::FixedPoint, intent::Intent};
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PublicIntentPublicBalanceBundle, SettlementBundle, SettlementObligation,
};

use crate::{
    darkpool_client::{DarkpoolClient, utils::u256_to_amount},
    indexer::{
        error::IndexerError,
        event_indexing::party_settlement_data::{
            PartySettlementData, parse_party_settlement_obligation,
        },
    },
    state_transitions::{StateTransition, create_public_intent::CreatePublicIntentTransition, settle_match_into_public_intent::SettleMatchIntoPublicIntentTransition},
};

// ---------
// | Types |
// ---------

/// Settlement data for a ring 0 (natively-settled, public-intent) settlement
pub struct Ring0SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: PublicIntentPublicBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

// --------------------------
// | Event Indexing Helpers |
// --------------------------

impl Ring0SettlementData {
    /// Get the state transition associated with the public intent creation
    /// event.
    ///
    /// Returns `None` if this party did not create the public intent.
    pub async fn get_state_transition_for_public_intent_creation(
        &self,
        darkpool_client: &DarkpoolClient,
        intent_hash: B256,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the intent hash doesn't match, this party did not create the public intent
        if self.get_intent_hash() != intent_hash {
            return Ok(None);
        }

        let intent = self.get_intent();
        let amount_in = self.get_amount_in();

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        Ok(Some(StateTransition::CreatePublicIntent(CreatePublicIntentTransition {
            intent,
            amount_in,
            intent_hash,
            block_number,
        })))
    }

    /// Get the state transition associated with the public intent update
    /// event.
    ///
    /// Returns `None` if this party did not update the public intent.
    pub async fn get_state_transition_for_public_intent_update(
        &self,
        darkpool_client: &DarkpoolClient,
        intent_hash: B256,
        version: u64,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the intent hash doesn't match, this party did not update the public intent
        if self.get_intent_hash() != intent_hash {
            return Ok(None);
        }

        let amount_in = self.get_amount_in();

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        Ok(Some(StateTransition::SettleMatchIntoPublicIntent(SettleMatchIntoPublicIntentTransition {
            amount_in,
            intent_hash,
            version,
            block_number,
        })))
    }
}

// ------------------
// | Member Helpers |
// ------------------

impl Ring0SettlementData {
    /// Parse ring 0 bundle data from the given settlement & obligation bundles
    pub fn new(
        settlement_bundle: &SettlementBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            PublicIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        Ok(Self { settlement_bundle: settlement_bundle_data, settlement_obligation })
    }

    /// Get the intent hash from the settlement bundle data
    pub fn get_intent_hash(&self) -> B256 {
        keccak256(self.settlement_bundle.auth.permit.abi_encode())
    }

    /// Get the intent from the settlement bundle data
    pub fn get_intent(&self) -> Intent {
        let sol_intent = &self.settlement_bundle.auth.permit.intent;

        let min_price = FixedPoint::from_repr(u256_to_scalar(&sol_intent.minPrice.repr));
        let amount_in = u256_to_amount(sol_intent.amountIn);

        Intent {
            in_token: sol_intent.inToken,
            out_token: sol_intent.outToken,
            owner: sol_intent.owner,
            min_price,
            amount_in,
        }
    }

    /// Get the input amount on the settlement obligation
    pub fn get_amount_in(&self) -> Amount {
        u256_to_amount(self.settlement_obligation.amountIn)
    }
}

// -------------------
// | Parsing Helpers |
// -------------------

/// Parse ring 0 bundle data & return a `PartySettlementData`
pub fn parse_ring0_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<PartySettlementData, IndexerError> {
    Ring0SettlementData::new(settlement_bundle, obligation_bundle, is_party0)
        .map(PartySettlementData::Ring0)
}
