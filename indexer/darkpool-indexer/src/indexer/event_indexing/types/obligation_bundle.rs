//! Defines a wrapper type & parsing utilities for the various kinds of
//! obligation bundles

use alloy::sol_types::SolValue;
use renegade_circuit_types::balance::PostMatchBalanceShare;
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateObligationBundle, SettlementObligation,
};

use crate::{
    indexer::{error::IndexerError, event_indexing::utils::to_circuit_post_match_balance_share},
    types::ObligationAmounts,
};

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
    /// Get the balance update data for the given party,
    /// if this is a private obligation bundle
    pub fn get_balance_shares_in_private_match(
        &self,
        is_party0: bool,
        is_input_balance: bool,
    ) -> Option<PostMatchBalanceShare> {
        let shares = match self {
            Self::Private(private_obligation_bundle) => match (is_party0, is_input_balance) {
                (true, true) => &private_obligation_bundle.statement.newInBalancePublicShares0,
                (true, false) => &private_obligation_bundle.statement.newOutBalancePublicShares0,
                (false, true) => &private_obligation_bundle.statement.newInBalancePublicShares1,
                (false, false) => &private_obligation_bundle.statement.newOutBalancePublicShares1,
            },
            _ => return None,
        };

        Some(to_circuit_post_match_balance_share(shares))
    }

    /// Get the updated public share of the intent amount for the given party,
    /// if this is a private obligation bundle
    pub fn get_updated_intent_amount_public_share(&self, is_party0: bool) -> Option<Scalar> {
        let updated_amount_share = match self {
            Self::Private(private_obligation_bundle) => {
                if is_party0 {
                    private_obligation_bundle.statement.newAmountPublicShare0
                } else {
                    private_obligation_bundle.statement.newAmountPublicShare1
                }
            },
            _ => return None,
        };

        Some(u256_to_scalar(&updated_amount_share))
    }

    /// Get the input & output amounts on the given party's obligation bundle,
    /// if this is a public obligation bundle
    pub fn get_public_obligation_amounts(&self, is_party0: bool) -> Option<ObligationAmounts> {
        let [amount_in, amount_out] = match self {
            Self::Public { party0_obligation, party1_obligation } => {
                if is_party0 {
                    [party0_obligation.amountIn, party1_obligation.amountOut]
                } else {
                    [party1_obligation.amountIn, party0_obligation.amountOut]
                }
            },
            _ => return None,
        }
        .each_ref()
        .map(u256_to_scalar);

        Some(ObligationAmounts { amount_in, amount_out })
    }
}
