//! Defines a wrapper type & parsing utilities for the data associated with the
//! different kinds of settlement types pertaining to one of the parties in a
//! match

use alloy::sol_types::SolValue;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ExistingBalanceBundle, NewBalanceBundle, ObligationBundle,
        PrivateIntentPublicBalanceBundle, PrivateIntentPublicBalanceFirstFillBundle,
        PrivateObligationBundle, PublicIntentPublicBalanceBundle, RenegadeSettledIntentBundle,
        RenegadeSettledIntentFirstFillBundle, RenegadeSettledPrivateFillBundle,
        RenegadeSettledPrivateFirstFillBundle, SettlementObligation, settleMatchCall,
    },
    calldata_bundles::{
        NATIVE_SETTLED_PRIVATE_INTENT_BUNDLE_TYPE, NATIVE_SETTLED_PUBLIC_INTENT_BUNDLE_TYPE,
        NATIVE_SETTLED_RENEGADE_PRIVATE_INTENT_BUNDLE_TYPE,
        RENEGADE_SETTLED_PRIVATE_FILL_BUNDLE_TYPE,
    },
};

use crate::indexer::{
    error::IndexerError,
    event_indexing::party_settlement_data::{
        ring0::parse_ring0_settlement_data, ring1::parse_ring1_settlement_data,
        ring2::parse_ring2_settlement_data, ring3::parse_ring3_settlement_data,
    },
};

pub mod ring0;
pub mod ring1;
pub mod ring2;
pub mod ring3;

/// The settlement bundle data for the party, including all decoded nested
/// fields & relevant fields from the obligation bundle
pub enum PartySettlementData {
    /// A natively-settled, public-intent bundle
    Ring0(PublicIntentPublicBalanceBundle, SettlementObligation),
    /// A natively-settled, private-intent first fill bundle
    Ring1FirstFill(PrivateIntentPublicBalanceFirstFillBundle, SettlementObligation),
    /// A natively-settled, private-intent bundle
    Ring1(PrivateIntentPublicBalanceBundle, SettlementObligation),
    /// A renegade-settled, public-fill intent first fill bundle into a new
    /// output balance
    Ring2FirstFillNewOutBalance(
        RenegadeSettledIntentFirstFillBundle,
        NewBalanceBundle,
        SettlementObligation,
    ),
    /// A renegade-settled, public-fill intent first fill bundle into an
    /// existing output balance
    Ring2FirstFill(
        RenegadeSettledIntentFirstFillBundle,
        ExistingBalanceBundle,
        SettlementObligation,
    ),
    /// A renegade-settled, public-fill intent bundle
    Ring2(RenegadeSettledIntentBundle, ExistingBalanceBundle, SettlementObligation),
    /// A renegade-settled, private-fill intent first fill bundle into a new
    /// output balance
    Ring3FirstFillNewOutBalance(
        RenegadeSettledPrivateFirstFillBundle,
        NewBalanceBundle,
        PrivateObligationBundle,
        bool, // is_party0
    ),
    /// A renegade-settled, private-fill intent first fill bundle into an
    /// existing output balance
    Ring3FirstFill(
        RenegadeSettledPrivateFirstFillBundle,
        ExistingBalanceBundle,
        PrivateObligationBundle,
        bool, // is_party0
    ),
    /// A renegade-settled, private-fill intent bundle
    Ring3(
        RenegadeSettledPrivateFillBundle,
        ExistingBalanceBundle,
        PrivateObligationBundle,
        bool, // is_party0
    ),
}

impl PartySettlementData {
    /// Parse the party settlement data from the given settle match call
    pub fn from_settle_match_call(
        is_party0: bool,
        settle_match_call: &settleMatchCall,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle = if is_party0 {
            &settle_match_call.party0SettlementBundle
        } else {
            &settle_match_call.party1SettlementBundle
        };

        let bundle_type = settlement_bundle.bundleType;

        match bundle_type {
            NATIVE_SETTLED_PUBLIC_INTENT_BUNDLE_TYPE => parse_ring0_settlement_data(
                settlement_bundle,
                &settle_match_call.obligationBundle,
                is_party0,
            ),
            NATIVE_SETTLED_PRIVATE_INTENT_BUNDLE_TYPE => parse_ring1_settlement_data(
                settlement_bundle,
                &settle_match_call.obligationBundle,
                is_party0,
                settlement_bundle.isFirstFill,
            ),
            NATIVE_SETTLED_RENEGADE_PRIVATE_INTENT_BUNDLE_TYPE => parse_ring2_settlement_data(
                settlement_bundle,
                &settle_match_call.obligationBundle,
                is_party0,
                settlement_bundle.isFirstFill,
            ),
            RENEGADE_SETTLED_PRIVATE_FILL_BUNDLE_TYPE => parse_ring3_settlement_data(
                settlement_bundle,
                &settle_match_call.obligationBundle,
                is_party0,
                settlement_bundle.isFirstFill,
            ),
            _ => Err(IndexerError::invalid_settlement_bundle(format!(
                "invalid settlement bundle type: {bundle_type}"
            ))),
        }
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Parse the given party's settlement obligation from the given obligation
/// bundle, assuming it is a public obligation bundle
pub fn parse_party_settlement_obligation(
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<SettlementObligation, IndexerError> {
    <(SettlementObligation, SettlementObligation) as SolValue>::abi_decode(&obligation_bundle.data)
        .map_err(IndexerError::parse)
        .map(
            |(party0_obligation, party1_obligation)| {
                if is_party0 { party0_obligation } else { party1_obligation }
            },
        )
}
