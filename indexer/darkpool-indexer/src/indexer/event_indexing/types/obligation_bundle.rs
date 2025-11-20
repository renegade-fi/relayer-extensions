//! Defines a wrapper type & parsing utilities for the various kinds of
//! obligation bundles

use alloy::{primitives::U256, sol_types::SolValue};
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateObligationBundle, SettlementObligation,
};

use crate::indexer::error::IndexerError;

// -------------
// | Constants |
// -------------

/// The value for the `PUBLIC` variant of the Solidity `ObligationType` enum
const PUBLIC_OBLIGATION: u8 = 0;
/// The value for the `PRIVATE` variant of the Solidity `ObligationType` enum
const PRIVATE_OBLIGATION: u8 = 1;

// -------------------------------
// | Obligation Bundle Data Type |
// -------------------------------

/// A wrapper around the different types of obligation bundle data
#[allow(clippy::large_enum_variant)]
pub enum ObligationBundleData {
    /// A public obligation bundle, containing a `SettlementObligation` struct
    /// for each party in a match
    Public {
        /// The settlement obligation for party 0
        party0_obligation: SettlementObligation,
        /// The settlement obligation for party 1
        party1_obligation: SettlementObligation,
    },
    /// A private obligation bundle, containing a single
    /// `PrivateObligationBundle` struct
    Private(PrivateObligationBundle),
}

impl TryFrom<&ObligationBundle> for ObligationBundleData {
    type Error = IndexerError;

    fn try_from(obligation_bundle: &ObligationBundle) -> Result<Self, Self::Error> {
        let obligation_type = obligation_bundle.obligationType;

        match obligation_type {
            PUBLIC_OBLIGATION => {
                <(SettlementObligation, SettlementObligation) as SolValue>::abi_decode(
                    &obligation_bundle.data,
                )
                .map_err(IndexerError::parse)
                .map(|(party0_obligation, party1_obligation)| {
                    ObligationBundleData::Public { party0_obligation, party1_obligation }
                })
            },
            PRIVATE_OBLIGATION => PrivateObligationBundle::abi_decode(&obligation_bundle.data)
                .map_err(IndexerError::parse)
                .map(ObligationBundleData::Private),
            _ => Err(IndexerError::invalid_settlement_bundle(format!(
                "invalid obligation bundle type: {obligation_type}"
            ))),
        }
    }
}

impl ObligationBundleData {
    /// Get the updated input/output balance public shares for the given party,
    /// if this is a private obligation bundle
    pub fn get_updated_balance_public_shares(
        &self,
        is_party0: bool,
        is_input_balance: bool,
    ) -> Option<[U256; 3]> {
        match self {
            Self::Private(private_obligation_bundle) => match (is_party0, is_input_balance) {
                (true, true) => Some(private_obligation_bundle.statement.newInBalancePublicShares0),
                (true, false) => {
                    Some(private_obligation_bundle.statement.newOutBalancePublicShares0)
                },
                (false, true) => {
                    Some(private_obligation_bundle.statement.newInBalancePublicShares1)
                },
                (false, false) => {
                    Some(private_obligation_bundle.statement.newOutBalancePublicShares1)
                },
            },
            _ => None,
        }
    }

    /// Get the updated public share of the intent amount for the given party,
    /// if this is a private obligation bundle
    pub fn get_updated_intent_amount_public_share(&self, is_party0: bool) -> Option<U256> {
        match self {
            Self::Private(private_obligation_bundle) => {
                if is_party0 {
                    Some(private_obligation_bundle.statement.newAmountPublicShare0)
                } else {
                    Some(private_obligation_bundle.statement.newAmountPublicShare1)
                }
            },
            _ => None,
        }
    }

    /// Get the input amount on the given party's obligation bundle, if this is
    /// a public obligation bundle
    pub fn get_amount_in(&self, is_party0: bool) -> Option<U256> {
        match self {
            Self::Public { party0_obligation, party1_obligation } => {
                if is_party0 {
                    Some(party0_obligation.amountIn)
                } else {
                    Some(party1_obligation.amountIn)
                }
            },
            _ => None,
        }
    }

    /// Get the output amount on the given party's obligation bundle, if this is
    /// a public obligation bundle
    pub fn get_amount_out(&self, is_party0: bool) -> Option<U256> {
        match self {
            Self::Public { party0_obligation, party1_obligation } => {
                if is_party0 {
                    Some(party0_obligation.amountOut)
                } else {
                    Some(party1_obligation.amountOut)
                }
            },
            _ => None,
        }
    }
}
