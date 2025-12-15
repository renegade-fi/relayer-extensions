//! Utilities for constructing & interacting with ring 2 settlement data

use alloy::sol_types::SolValue;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ExistingBalanceBundle, NewBalanceBundle, ObligationBundle, RenegadeSettledIntentBundle,
        RenegadeSettledIntentFirstFillBundle, SettlementBundle, SettlementObligation,
    },
    calldata_bundles::{EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE, NEW_OUTPUT_BALANCE_BUNDLE_TYPE},
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

/// Settlement data for a ring 2 (renegade-settled, public-fill) settlement
/// representing the first fill on the party's intent, into a new output balance
pub struct Ring2FirstFillNewOutBalanceSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledIntentFirstFillBundle,
    /// The new output balance bundle
    pub new_balance_bundle: NewBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

impl Ring2FirstFillNewOutBalanceSettlementData {
    /// Parse ring 2 first fill new output balance bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledIntentFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        let new_balance_bundle =
            NewBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        Ok(Ring2FirstFillNewOutBalanceSettlementData {
            settlement_bundle: settlement_bundle_data,
            new_balance_bundle,
            settlement_obligation,
        })
    }
}

/// Settlement data for a ring 2 (renegade-settled, public-fill) settlement
/// representing the first fill on the party's intent, into an existing output
/// balance
pub struct Ring2FirstFillSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledIntentFirstFillBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

impl Ring2FirstFillSettlementData {
    /// Parse ring 2 first fill bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledIntentFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        Ok(Ring2FirstFillSettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            settlement_obligation,
        })
    }
}

/// Settlement data for a ring 2 (renegade-settled, public-fill) settlement that
/// was not the first fill on the party's intent
pub struct Ring2SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledIntentBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

impl Ring2SettlementData {
    /// Parse ring 2 bundle data from the given settlement & obligation bundles.
    pub fn new(
        settlement_bundle: &SettlementBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            RenegadeSettledIntentBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        Ok(Ring2SettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            settlement_obligation,
        })
    }
}

// -------------------
// | Parsing Helpers |
// -------------------

/// Parse ring 2 settlement data from the given settlement & obligation bundles
pub fn parse_ring2_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
    is_first_fill: bool,
) -> Result<PartySettlementData, IndexerError> {
    if !is_first_fill {
        return Ring2SettlementData::new(settlement_bundle, obligation_bundle, is_party0)
            .map(PartySettlementData::Ring2);
    }

    let settlement_bundle_data =
        RenegadeSettledIntentFirstFillBundle::abi_decode(&settlement_bundle.data)
            .map_err(IndexerError::parse)?;

    let output_bundle_type = settlement_bundle_data.outputBalanceBundle.bundleType;

    match output_bundle_type {
        EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE => {
            Ring2FirstFillSettlementData::new(settlement_bundle_data, obligation_bundle, is_party0)
                .map(PartySettlementData::Ring2FirstFill)
        },
        NEW_OUTPUT_BALANCE_BUNDLE_TYPE => Ring2FirstFillNewOutBalanceSettlementData::new(
            settlement_bundle_data,
            obligation_bundle,
            is_party0,
        )
        .map(PartySettlementData::Ring2FirstFillNewOutBalance),
        _ => Err(IndexerError::invalid_output_balance_bundle(format!(
            "invalid output balance bundle type: {}",
            output_bundle_type
        ))),
    }
}
