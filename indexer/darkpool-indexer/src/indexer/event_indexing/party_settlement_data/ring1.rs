//! Utilities for constructing & interacting with ring 1 settlement data

use alloy::sol_types::SolValue;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateIntentPublicBalanceBundle, PrivateIntentPublicBalanceFirstFillBundle,
    SettlementBundle,
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

/// Parse ring 1 first fill bundle data from the given settlement & obligation
/// bundles
pub fn parse_ring1_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
    is_first_fill: bool,
) -> Result<PartySettlementData, IndexerError> {
    let settlement_obligation = parse_party_settlement_obligation(obligation_bundle, is_party0)?;

    if is_first_fill {
        let settlement_bundle_data =
            PrivateIntentPublicBalanceFirstFillBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        Ok(PartySettlementData::Ring1FirstFill(settlement_bundle_data, settlement_obligation))
    } else {
        let settlement_bundle_data =
            PrivateIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        Ok(PartySettlementData::Ring1(settlement_bundle_data, settlement_obligation))
    }
}
