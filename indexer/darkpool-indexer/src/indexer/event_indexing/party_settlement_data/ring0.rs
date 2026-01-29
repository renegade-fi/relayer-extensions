//! Utilities for constructing & interacting with ring 0 settlement data

use alloy::{
    primitives::{B256, TxHash, U256, keccak256},
    sol_types::SolValue,
};
use renegade_circuit_types::{Amount, fixed_point::FixedPoint};
use renegade_crypto::fields::u256_to_scalar;
use renegade_darkpool_types::intent::Intent;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    BoundedMatchResult, ObligationBundle, PublicIntentPublicBalanceBundle, SettlementBundle,
    SettlementObligation,
};

use crate::{
    darkpool_client::{DarkpoolClient, utils::u256_to_amount},
    indexer::{
        error::IndexerError,
        event_indexing::party_settlement_data::{
            PartySettlementData, parse_party_settlement_obligation,
        },
    },
    state_transitions::{
        StateTransition,
        create_public_intent::{CreatePublicIntentTransition, PublicIntentCreationData},
        settle_match_into_public_intent::{
            PublicIntentSettlementData, SettleMatchIntoPublicIntentTransition,
        },
    },
};

/// Settlement data for a ring 0 (natively-settled, public-intent) settlement
pub struct Ring0SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: PublicIntentPublicBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

// --- Public API ---
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

        let public_intent_creation_data =
            PublicIntentCreationData::InternalMatch { intent, amount_in };

        Ok(Some(StateTransition::CreatePublicIntent(CreatePublicIntentTransition {
            intent_hash,
            tx_hash,
            block_number,
            public_intent_creation_data,
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
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the intent hash doesn't match, this party did not update the public intent
        if self.get_intent_hash() != intent_hash {
            return Ok(None);
        }

        let amount_in = self.get_amount_in();
        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        let public_intent_settlement_data = PublicIntentSettlementData::InternalMatch { amount_in };

        Ok(Some(StateTransition::SettleMatchIntoPublicIntent(
            SettleMatchIntoPublicIntentTransition {
                intent_hash,
                tx_hash,
                block_number,
                public_intent_settlement_data,
            },
        )))
    }
}

// -- Private Helpers ---
impl Ring0SettlementData {
    /// Get the intent hash
    fn get_intent_hash(&self) -> B256 {
        keccak256(self.settlement_bundle.auth.intentPermit.abi_encode())
    }

    /// Get the intent
    fn get_intent(&self) -> Intent {
        let sol_intent = &self.settlement_bundle.auth.intentPermit.intent;

        let min_price = FixedPoint::from_repr(u256_to_scalar(sol_intent.minPrice.repr));
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
    fn get_amount_in(&self) -> Amount {
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

/// Settlement data for a ring 0 external match (public intent settled with
/// external party)
pub struct Ring0ExternalSettlementData {
    /// The settlement bundle data (parsed same as Ring0SettlementData)
    pub settlement_bundle: PublicIntentPublicBalanceBundle,
    /// The bounded match result from the external match call
    pub match_result: BoundedMatchResult,
    /// The external party's input amount
    pub external_party_amount_in: U256,
}

// --- Public API ---
impl Ring0ExternalSettlementData {
    /// Parse ring 0 external match settlement data from the given settlement
    /// bundle and match result
    pub fn new(
        settlement_bundle: &SettlementBundle,
        match_result: BoundedMatchResult,
        external_party_amount_in: U256,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            PublicIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        Ok(Self {
            settlement_bundle: settlement_bundle_data,
            match_result,
            external_party_amount_in,
        })
    }

    /// Get the intent hash
    pub fn get_intent_hash(&self) -> B256 {
        keccak256(self.settlement_bundle.auth.intentPermit.abi_encode())
    }

    /// Get the intent
    pub fn get_intent(&self) -> Intent {
        let sol_intent = &self.settlement_bundle.auth.intentPermit.intent;

        let min_price = sol_intent.minPrice.clone().into();
        let amount_in = u256_to_amount(sol_intent.amountIn);

        Intent {
            in_token: sol_intent.inToken,
            out_token: sol_intent.outToken,
            owner: sol_intent.owner,
            min_price,
            amount_in,
        }
    }

    /// Get the price from the match result
    pub fn get_price(&self) -> FixedPoint {
        self.match_result.price.clone().into()
    }

    /// Get the external party's input amount
    pub fn get_external_party_amount_in(&self) -> Amount {
        u256_to_amount(self.external_party_amount_in)
    }

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
        let price = self.get_price();
        let external_party_amount_in = self.get_external_party_amount_in();
        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        let public_intent_creation_data =
            PublicIntentCreationData::ExternalMatch { intent, price, external_party_amount_in };

        Ok(Some(StateTransition::CreatePublicIntent(CreatePublicIntentTransition {
            intent_hash,
            tx_hash,
            block_number,
            public_intent_creation_data,
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
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the intent hash doesn't match, this party did not update the public intent
        if self.get_intent_hash() != intent_hash {
            return Ok(None);
        }

        let price = self.get_price();
        let external_party_amount_in = self.get_external_party_amount_in();
        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        let public_intent_settlement_data =
            PublicIntentSettlementData::ExternalMatch { price, external_party_amount_in };

        Ok(Some(StateTransition::SettleMatchIntoPublicIntent(
            SettleMatchIntoPublicIntentTransition {
                intent_hash,
                tx_hash,
                block_number,
                public_intent_settlement_data,
            },
        )))
    }
}
