//! Utilities for constructing & interacting with ring 1 settlement data

use alloy::{primitives::TxHash, sol_types::SolValue};
use renegade_circuit_types::{
    Nullifier, intent::IntentShare,
    settlement_obligation::SettlementObligation as CircuitSettlementObligation,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateIntentPublicBalanceBundle, PrivateIntentPublicBalanceFirstFillBundle,
    SettlementBundle, SettlementObligation,
};

use crate::{
    darkpool_client::DarkpoolClient,
    indexer::{
        error::IndexerError,
        event_indexing::party_settlement_data::{
            PartySettlementData, parse_party_settlement_obligation,
        },
    },
    state_transitions::{
        StateTransition,
        create_intent::{CreateIntentTransition, IntentCreationData},
        settle_match_into_intent::{IntentSettlementData, SettleMatchIntoIntentTransition},
    },
};

/// Settlement data for a ring 1 (natively-settled, private-intent) settlement
/// representing the first fill on the party's intent
pub struct Ring1FirstFillSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: PrivateIntentPublicBalanceFirstFillBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

// --- Public API ---
impl Ring1FirstFillSettlementData {
    /// Parse ring 1 first fill bundle data from the given settlement &
    /// obligation bundles
    pub fn new(
        settlement_bundle: &SettlementBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            PrivateIntentPublicBalanceFirstFillBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        Ok(Self { settlement_bundle: settlement_bundle_data, settlement_obligation })
    }

    /// Get the state transition associated with the recovery ID event.
    ///
    /// Returns `None` if this party did not create a new intent with the given
    /// recovery ID.
    pub async fn get_state_transition_for_recovery_id(
        &self,
        darkpool_client: &DarkpoolClient,
        recovery_id: Scalar,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the given recovery ID doesn't match that of the newly-created intent
        // in this bundle, we don't produce a state transition for this event.
        if self.get_intent_recovery_id() != recovery_id {
            return Ok(None);
        }

        let pre_match_full_intent_share = self.get_intent_share();
        let settlement_obligation = self.get_settlement_obligation();

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        let intent_creation_data =
            IntentCreationData::PublicFill { pre_match_full_intent_share, settlement_obligation };

        Ok(Some(StateTransition::CreateIntent(CreateIntentTransition {
            recovery_id,
            block_number,
            intent_creation_data,
        })))
    }
}

// -- Private Helpers ---
impl Ring1FirstFillSettlementData {
    /// Get the intent recovery ID from the settlement bundle data
    fn get_intent_recovery_id(&self) -> Scalar {
        u256_to_scalar(&self.settlement_bundle.auth.statement.recoveryId)
    }

    /// Get the pre-match, full public sharing of this party's new intent
    fn get_intent_share(&self) -> IntentShare {
        self.settlement_bundle.auth.statement.intentPublicShare.clone().into()
    }

    /// Get the settlement obligation
    fn get_settlement_obligation(&self) -> CircuitSettlementObligation {
        self.settlement_obligation.clone().into()
    }
}

/// Settlement data for a ring 1 (natively-settled, private-intent) settlement
/// that was not the first fill on the party's intent
pub struct Ring1SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: PrivateIntentPublicBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

// --- Public API ---
impl Ring1SettlementData {
    /// Parse ring 1 bundle data from the given settlement & obligation bundles
    pub fn new(
        settlement_bundle: &SettlementBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            PrivateIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        Ok(Self { settlement_bundle: settlement_bundle_data, settlement_obligation })
    }

    /// Get the state transition associated with the nullifier event.
    ///
    /// Returns `None` if this party did not settle a match into their intent.
    pub async fn get_state_transition_for_nullifier(
        &self,
        darkpool_client: &DarkpoolClient,
        nullifier: Nullifier,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the given nullifier doesn't match the intent nullifier spent by this
        // party, we don't produce a state transition for this event.
        if self.get_intent_nullifier() != nullifier {
            return Ok(None);
        }

        let settlement_obligation = self.get_settlement_obligation();
        let intent_settlement_data = IntentSettlementData::PublicFill { settlement_obligation };

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        Ok(Some(StateTransition::SettleMatchIntoIntent(SettleMatchIntoIntentTransition {
            nullifier,
            block_number,
            intent_settlement_data,
        })))
    }
}

// -- Private Helpers ---
impl Ring1SettlementData {
    /// Get the spent intent nullifier from the settlement bundle data
    pub fn get_intent_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.settlement_bundle.auth.statement.oldIntentNullifier)
    }

    /// Get the settlement obligation
    pub fn get_settlement_obligation(&self) -> CircuitSettlementObligation {
        self.settlement_obligation.clone().into()
    }
}

// -------------------
// | Parsing Helpers |
// -------------------

/// Parse ring 1 first fill bundle data from the given settlement & obligation
/// bundles
pub fn parse_ring1_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
    is_first_fill: bool,
) -> Result<PartySettlementData, IndexerError> {
    if is_first_fill {
        Ring1FirstFillSettlementData::new(settlement_bundle, obligation_bundle, is_party0)
            .map(PartySettlementData::Ring1FirstFill)
    } else {
        Ring1SettlementData::new(settlement_bundle, obligation_bundle, is_party0)
            .map(PartySettlementData::Ring1)
    }
}
