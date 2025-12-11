//! Defines a wrapper type & parsing utilities for the various kinds of
//! obligation bundles

use alloy::{primitives::Address, sol_types::SolValue};
use renegade_circuit_types::balance::PostMatchBalanceShare;
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateObligationBundle, SettlementObligation,
};

use crate::{indexer::error::IndexerError, types::ObligationAmounts};

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
                (true, true) => {
                    private_obligation_bundle.statement.newInBalancePublicShares0.clone()
                },
                (true, false) => {
                    private_obligation_bundle.statement.newOutBalancePublicShares0.clone()
                },
                (false, true) => {
                    private_obligation_bundle.statement.newInBalancePublicShares1.clone()
                },
                (false, false) => {
                    private_obligation_bundle.statement.newOutBalancePublicShares1.clone()
                },
            },
            _ => return None,
        };

        Some(shares.into())
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

    /// Get the settlement obligation for the given party, if this is a public
    /// obligation bundle
    pub fn get_public_settlement_obligation(
        &self,
        is_party0: bool,
    ) -> Option<SettlementObligation> {
        match self {
            Self::Public { party0_obligation, party1_obligation } => {
                if is_party0 {
                    Some(party0_obligation.clone())
                } else {
                    Some(party1_obligation.clone())
                }
            },
            _ => None,
        }
    }

    /// Get the input & output amounts on the given party's obligation bundle,
    /// if this is a public obligation bundle
    pub fn get_public_obligation_amounts(&self, is_party0: bool) -> Option<ObligationAmounts> {
        self.get_public_settlement_obligation(is_party0).map(|settlement_obligation| {
            let amount_in = u256_to_scalar(&settlement_obligation.amountIn);
            let amount_out = u256_to_scalar(&settlement_obligation.amountOut);
            ObligationAmounts { amount_in, amount_out }
        })
    }

    /// Get the pair traded, if this is a public obligation bundle
    pub fn get_public_obligation_trading_pair(&self) -> Option<(Address, Address)> {
        match self {
            Self::Public { party0_obligation, .. } => {
                Some((party0_obligation.inputToken, party0_obligation.outputToken))
            },
            _ => None,
        }
    }
}
