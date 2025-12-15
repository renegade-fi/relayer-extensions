//! Defines a wrapper type & parsing utilities for the data associated with the
//! different kinds of settlement types pertaining to one of the parties in a
//! match

use alloy::{
    primitives::{B256, TxHash},
    sol_types::SolValue,
};
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{ObligationBundle, SettlementObligation, settleMatchCall},
    calldata_bundles::{
        NATIVE_SETTLED_PRIVATE_INTENT_BUNDLE_TYPE, NATIVE_SETTLED_PUBLIC_INTENT_BUNDLE_TYPE,
        NATIVE_SETTLED_RENEGADE_PRIVATE_INTENT_BUNDLE_TYPE,
        RENEGADE_SETTLED_PRIVATE_FILL_BUNDLE_TYPE,
    },
};

use crate::{
    darkpool_client::DarkpoolClient,
    indexer::{
        error::IndexerError,
        event_indexing::party_settlement_data::{
            ring0::{Ring0SettlementData, parse_ring0_settlement_data},
            ring1::{
                Ring1FirstFillSettlementData, Ring1SettlementData, parse_ring1_settlement_data,
            },
            ring2::{
                Ring2FirstFillNewOutBalanceSettlementData, Ring2FirstFillSettlementData,
                Ring2SettlementData, parse_ring2_settlement_data,
            },
            ring3::{
                Ring3FirstFillNewOutBalanceSettlementData, Ring3FirstFillSettlementData,
                Ring3SettlementData, parse_ring3_settlement_data,
            },
        },
    },
    state_transitions::StateTransition,
};

pub mod ring0;
pub mod ring1;
pub mod ring2;
pub mod ring3;

/// The settlement bundle data for the party, including all decoded nested
/// fields & relevant fields from the obligation bundle
pub enum PartySettlementData {
    /// A natively-settled, public-intent bundle
    Ring0(Ring0SettlementData),
    /// A natively-settled, private-intent first fill bundle
    Ring1FirstFill(Ring1FirstFillSettlementData),
    /// A natively-settled, private-intent bundle
    Ring1(Ring1SettlementData),
    /// A renegade-settled, public-fill intent first fill bundle into a new
    /// output balance
    Ring2FirstFillNewOutBalance(Ring2FirstFillNewOutBalanceSettlementData),
    /// A renegade-settled, public-fill intent first fill bundle into an
    /// existing output balance
    Ring2FirstFill(Ring2FirstFillSettlementData),
    /// A renegade-settled, public-fill intent bundle
    Ring2(Ring2SettlementData),
    /// A renegade-settled, private-fill intent first fill bundle into a new
    /// output balance
    Ring3FirstFillNewOutBalance(Ring3FirstFillNewOutBalanceSettlementData),
    /// A renegade-settled, private-fill intent first fill bundle into an
    /// existing output balance
    Ring3FirstFill(Ring3FirstFillSettlementData),
    /// A renegade-settled, private-fill intent bundle
    Ring3(Ring3SettlementData),
}

// --------------------------
// | Event Indexing Helpers |
// --------------------------

impl PartySettlementData {
    pub async fn get_state_transition_for_public_intent_creation(
        &self,
        darkpool_client: &DarkpoolClient,
        intent_hash: B256,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        match self {
            Self::Ring0(ring0_settlement_data) => {
                ring0_settlement_data
                    .get_state_transition_for_public_intent_creation(
                        darkpool_client,
                        intent_hash,
                        tx_hash,
                    )
                    .await
            },
            _ => {
                Err(IndexerError::invalid_party_settlement_data("expected ring 0 settlement data"))
            },
        }
    }
}

// ------------------
// | Member Helpers |
// ------------------

impl PartySettlementData {
    /// Parse the party settlement data from the given settle match call
    pub fn from_settle_match_call(
        settle_match_call: &settleMatchCall,
        is_party0: bool,
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
