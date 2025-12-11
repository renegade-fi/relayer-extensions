//! Defines a wrapper type & parsing utilities for the various kinds of output
//! balance bundles

use alloy::sol_types::SolValue;
use renegade_circuit_types::{Nullifier, balance::PreMatchBalanceShare};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ExistingBalanceBundle, NewBalanceBundle, OutputBalanceBundle,
};

use crate::indexer::error::IndexerError;

// -------------
// | Constants |
// -------------

/// The value for the `EXISTING_BALANCE` variant of the Solidity
/// `OutputBalanceBundleType` enum
const EXISTING_BALANCE: u8 = 0;
/// The value for the `NEW_BALANCE` variant of the Solidity
/// `OutputBalanceBundleType` enum
const NEW_BALANCE: u8 = 1;

// -----------------------------------
// | Output Balance Bundle Data Type |
// -----------------------------------

/// A wrapper around the different types of output balance bundle data
#[derive(Clone)]
pub enum OutputBalanceBundleData {
    /// An output balance bundle for an existing balance
    ExistingBalanceBundle(ExistingBalanceBundle),
    /// An output balance bundle for a new balance
    NewBalanceBundle(NewBalanceBundle),
}

impl TryFrom<&OutputBalanceBundle> for OutputBalanceBundleData {
    type Error = IndexerError;

    fn try_from(output_balance_bundle: &OutputBalanceBundle) -> Result<Self, Self::Error> {
        let bundle_type = output_balance_bundle.bundleType;

        match bundle_type {
            EXISTING_BALANCE => ExistingBalanceBundle::abi_decode(&output_balance_bundle.data)
                .map_err(IndexerError::parse)
                .map(OutputBalanceBundleData::ExistingBalanceBundle),
            NEW_BALANCE => NewBalanceBundle::abi_decode(&output_balance_bundle.data)
                .map_err(IndexerError::parse)
                .map(OutputBalanceBundleData::NewBalanceBundle),
            _ => Err(IndexerError::invalid_output_balance_bundle(format!(
                "invalid output balance bundle type: {bundle_type}"
            ))),
        }
    }
}

impl OutputBalanceBundleData {
    /// Get the balance nullifier from the output balance bundle data, if one
    /// was spent
    pub fn get_balance_nullifier(&self) -> Option<Nullifier> {
        match self {
            Self::ExistingBalanceBundle(bundle) => {
                Some(u256_to_scalar(&bundle.statement.oldBalanceNullifier))
            },
            Self::NewBalanceBundle(_) => None,
        }
    }

    /// Get the recovery ID of the balance from the output balance bundle data
    pub fn get_balance_recovery_id(&self) -> Scalar {
        match self {
            Self::ExistingBalanceBundle(bundle) => u256_to_scalar(&bundle.statement.recoveryId),
            Self::NewBalanceBundle(bundle) => u256_to_scalar(&bundle.statement.recoveryId),
        }
    }

    /// Get the pre-match balance shares from the output balance bundle data, if
    /// it was a new balance bundle
    pub fn get_pre_match_balance_shares(&self) -> Option<PreMatchBalanceShare> {
        match self {
            Self::NewBalanceBundle(bundle) => {
                Some(bundle.statement.preMatchBalanceShares.clone().into())
            },
            Self::ExistingBalanceBundle(_) => None,
        }
    }
}
