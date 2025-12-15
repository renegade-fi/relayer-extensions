//! Utilities for constructing & interacting with ring 1 settlement data

use alloy::sol_types::SolValue;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateIntentPublicBalanceBundle, PrivateIntentPublicBalanceFirstFillBundle,
    SettlementBundle, SettlementObligation,
};

use crate::indexer::{
    error::IndexerError,
    event_indexing::party_settlement_data::{
        PartySettlementData, parse_party_settlement_obligation,
    },
};

// ---------
// | Types |
// ---------

/// Settlement data for a ring 1 (natively-settled, private-intent) settlement
/// representing the first fill on the party's intent
pub struct Ring1FirstFillSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: PrivateIntentPublicBalanceFirstFillBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

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
}

/// Settlement data for a ring 1 (natively-settled, private-intent) settlement
/// that was not the first fill on the party's intent
pub struct Ring1SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: PrivateIntentPublicBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

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
