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

// ---------
// | Types |
// ---------

/// Settlement data for a ring 3 (renegade-settled, private-fill) settlement
/// representing the first fill on the party's intent, into a new output balance
pub struct Ring3FirstFillNewOutBalanceSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledPrivateFirstFillBundle,
    /// The new output balance bundle
    pub new_balance_bundle: NewBalanceBundle,
    /// The private obligation bundle
    pub obligation_bundle: PrivateObligationBundle,
    /// Whether the party is party 0
    pub is_party0: bool,
}

impl Ring3FirstFillNewOutBalanceSettlementData {
    /// Parse ring 3 first fill new output balance bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledPrivateFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let new_balance_bundle =
            NewBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
            .map_err(IndexerError::parse)?;

        Ok(Ring3FirstFillNewOutBalanceSettlementData {
            settlement_bundle: settlement_bundle_data,
            new_balance_bundle,
            obligation_bundle,
            is_party0,
        })
    }
}

/// Settlement data for a ring 3 (renegade-settled, private-fill) settlement
/// representing the first fill on the party's intent, into an existing output
/// balance
pub struct Ring3FirstFillSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledPrivateFirstFillBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The private obligation bundle
    pub obligation_bundle: PrivateObligationBundle,
    /// Whether the party is party 0
    pub is_party0: bool,
}

impl Ring3FirstFillSettlementData {
    /// Parse ring 3 first fill bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledPrivateFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
            .map_err(IndexerError::parse)?;

        Ok(Ring3FirstFillSettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            obligation_bundle,
            is_party0,
        })
    }
}

/// Settlement data for a ring 3 (renegade-settled, private-fill) settlement
/// that was not the first fill on the party's intent
pub struct Ring3SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledPrivateFillBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The private obligation bundle
    pub obligation_bundle: PrivateObligationBundle,
    /// Whether the party is party 0
    pub is_party0: bool,
}

impl Ring3SettlementData {
    /// Parse ring 3 bundle data from the given settlement & obligation bundles.
    pub fn new(
        settlement_bundle: &SettlementBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            RenegadeSettledPrivateFillBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
            .map_err(IndexerError::parse)?;

        Ok(Ring3SettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            obligation_bundle,
            is_party0,
        })
    }
}

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
    if !is_first_fill {
        return Ring3SettlementData::new(settlement_bundle, obligation_bundle, is_party0)
            .map(PartySettlementData::Ring3);
    }

    let settlement_bundle_data =
        RenegadeSettledPrivateFirstFillBundle::abi_decode(&settlement_bundle.data)
            .map_err(IndexerError::parse)?;

    let output_bundle_type = settlement_bundle_data.outputBalanceBundle.bundleType;

    match output_bundle_type {
        EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE => {
            Ring3FirstFillSettlementData::new(settlement_bundle_data, obligation_bundle, is_party0)
                .map(PartySettlementData::Ring3FirstFill)
        },
        NEW_OUTPUT_BALANCE_BUNDLE_TYPE => Ring3FirstFillNewOutBalanceSettlementData::new(
            settlement_bundle_data,
            obligation_bundle,
            is_party0,
        )
        .map(PartySettlementData::Ring3FirstFillNewOutBalance),
        _ => Err(IndexerError::invalid_output_balance_bundle(format!(
            "invalid output balance bundle type: {}",
            output_bundle_type
        ))),
    }
}
