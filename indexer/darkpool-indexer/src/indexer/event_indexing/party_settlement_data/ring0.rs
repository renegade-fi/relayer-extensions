//! Utilities for constructing & interacting with ring 0 settlement data

use alloy::sol_types::SolValue;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PublicIntentPublicBalanceBundle, SettlementBundle,
};

use crate::indexer::{
    error::IndexerError,
    event_indexing::party_settlement_data::{
        PartySettlementData, parse_party_settlement_obligation,
    },
};

// -------------------
// | Parsing Helpers |
// -------------------

/// Parse ring 0 bundle data from the given settlement & obligation bundles
pub fn parse_ring0_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<PartySettlementData, IndexerError> {
    let settlement_bundle_data =
        PublicIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
            .map_err(IndexerError::parse)?;

    let settlement_obligation = parse_party_settlement_obligation(obligation_bundle, is_party0)?;

    Ok(PartySettlementData::Ring0(settlement_bundle_data, settlement_obligation))
}
