//! Utilities for constructing & interacting with ring 3 settlement data

use alloy::sol_types::SolValue;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ExistingBalanceBundle, NewBalanceBundle, ObligationBundle, PrivateObligationBundle,
        RenegadeSettledPrivateFillBundle, RenegadeSettledPrivateFirstFillBundle, SettlementBundle,
    },
    calldata_bundles::{EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE, NEW_OUTPUT_BALANCE_BUNDLE_TYPE},
};

use crate::indexer::{
    error::IndexerError, event_indexing::party_settlement_data::PartySettlementData,
};

// -------------------
// | Parsing Helpers |
// -------------------

/// Parse ring 3 settlement data from the given settlement & obligation bundles
pub fn parse_ring3_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
    is_first_fill: bool,
) -> Result<PartySettlementData, IndexerError> {
    let private_obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
        .map_err(IndexerError::parse)?;

    if !is_first_fill {
        let settlement_bundle_data =
            RenegadeSettledPrivateFillBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        return Ok(PartySettlementData::Ring3(
            settlement_bundle_data,
            existing_balance_bundle,
            private_obligation_bundle,
            is_party0,
        ));
    }

    let settlement_bundle_data =
        RenegadeSettledPrivateFirstFillBundle::abi_decode(&settlement_bundle.data)
            .map_err(IndexerError::parse)?;

    let output_bundle_type = settlement_bundle_data.outputBalanceBundle.bundleType;
    let output_bundle_bytes = &settlement_bundle_data.outputBalanceBundle.data;

    match output_bundle_type {
        EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE => {
            let existing_balance_bundle = ExistingBalanceBundle::abi_decode(output_bundle_bytes)
                .map_err(IndexerError::parse)?;

            Ok(PartySettlementData::Ring3FirstFill(
                settlement_bundle_data,
                existing_balance_bundle,
                private_obligation_bundle,
                is_party0,
            ))
        },
        NEW_OUTPUT_BALANCE_BUNDLE_TYPE => {
            let new_balance_bundle =
                NewBalanceBundle::abi_decode(output_bundle_bytes).map_err(IndexerError::parse)?;

            Ok(PartySettlementData::Ring3FirstFillNewOutBalance(
                settlement_bundle_data,
                new_balance_bundle,
                private_obligation_bundle,
                is_party0,
            ))
        },
        _ => Err(IndexerError::invalid_output_balance_bundle(format!(
            "invalid output balance bundle type: {}",
            output_bundle_type
        ))),
    }
}
