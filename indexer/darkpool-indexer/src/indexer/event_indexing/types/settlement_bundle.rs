//! Defines a wrapper type & parsing utilities for the various kinds of
//! settlement bundles

use alloy::{
    primitives::{B256, keccak256},
    sol_types::SolValue,
};
use renegade_circuit_types::{
    Nullifier,
    balance::{PostMatchBalanceShare, PreMatchBalanceShare},
    fixed_point::FixedPoint,
    intent::Intent,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    PrivateIntentPublicBalanceBundle, PrivateIntentPublicBalanceFirstFillBundle,
    PublicIntentPublicBalanceBundle, RenegadeSettledIntentBundle,
    RenegadeSettledIntentFirstFillBundle, RenegadeSettledPrivateFillBundle,
    RenegadeSettledPrivateFirstFillBundle, SettlementBundle,
};

use crate::{
    darkpool_client::utils::u256_to_amount,
    indexer::{
        error::IndexerError, event_indexing::types::output_balance_bundle::OutputBalanceBundleData,
    },
};

// -------------
// | Constants |
// -------------

/// The value for the `NATIVELY_SETTLED_PUBLIC_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
pub const NATIVELY_SETTLED_PUBLIC_INTENT: u8 = 0;
/// The value for the `NATIVELY_SETTLED_PRIVATE_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const NATIVELY_SETTLED_PRIVATE_INTENT: u8 = 1;
/// The value for the `RENEGADE_SETTLED_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const RENEGADE_SETTLED_INTENT: u8 = 2;
/// The value for the `RENEGADE_SETTLED_PRIVATE_FILL` variant of the Solidity
/// `SettlementBundleType` enum
const RENEGADE_SETTLED_PRIVATE_FILL: u8 = 3;

// -------------------------------
// | Settlement Bundle Data Type |
// -------------------------------

/// A wrapper around the different types of settlement bundle data
pub enum SettlementBundleData {
    /// A natively-settled, public-intent bundle
    PublicIntentPublicBalance(PublicIntentPublicBalanceBundle),
    /// A natively-settled, private-intent first fill bundle
    PrivateIntentPublicBalanceFirstFill(PrivateIntentPublicBalanceFirstFillBundle),
    /// A natively-settled, private-intent bundle
    PrivateIntentPublicBalance(PrivateIntentPublicBalanceBundle),
    /// A renegade-settled, public-fill intent first fill bundle
    RenegadeSettledIntentFirstFill(RenegadeSettledIntentFirstFillBundle),
    /// A renegade-settled, public-fill intent bundle
    RenegadeSettledIntent(RenegadeSettledIntentBundle),
    /// A renegade-settled, private-fill intent first fill bundle
    RenegadeSettledPrivateFirstFill(RenegadeSettledPrivateFirstFillBundle),
    /// A renegade-settled, private-fill intent bundle
    RenegadeSettledPrivateFill(RenegadeSettledPrivateFillBundle),
}

impl TryFrom<&SettlementBundle> for SettlementBundleData {
    type Error = IndexerError;

    fn try_from(settlement_bundle: &SettlementBundle) -> Result<Self, Self::Error> {
        let is_first_fill = settlement_bundle.isFirstFill;
        let bundle_type = settlement_bundle.bundleType;

        match bundle_type {
            // Natively-settled bundles don't spend a balance state object's nullifier
            NATIVELY_SETTLED_PUBLIC_INTENT => {
                PublicIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                    .map_err(IndexerError::parse)
                    .map(SettlementBundleData::PublicIntentPublicBalance)
            },
            NATIVELY_SETTLED_PRIVATE_INTENT => {
                if is_first_fill {
                    PrivateIntentPublicBalanceFirstFillBundle::abi_decode(&settlement_bundle.data)
                        .map_err(IndexerError::parse)
                        .map(SettlementBundleData::PrivateIntentPublicBalanceFirstFill)
                } else {
                    PrivateIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                        .map_err(IndexerError::parse)
                        .map(SettlementBundleData::PrivateIntentPublicBalance)
                }
            },
            RENEGADE_SETTLED_INTENT => {
                if is_first_fill {
                    RenegadeSettledIntentFirstFillBundle::abi_decode(&settlement_bundle.data)
                        .map_err(IndexerError::parse)
                        .map(SettlementBundleData::RenegadeSettledIntentFirstFill)
                } else {
                    RenegadeSettledIntentBundle::abi_decode(&settlement_bundle.data)
                        .map_err(IndexerError::parse)
                        .map(SettlementBundleData::RenegadeSettledIntent)
                }
            },
            RENEGADE_SETTLED_PRIVATE_FILL => {
                if is_first_fill {
                    RenegadeSettledPrivateFirstFillBundle::abi_decode(&settlement_bundle.data)
                        .map_err(IndexerError::parse)
                        .map(SettlementBundleData::RenegadeSettledPrivateFirstFill)
                } else {
                    RenegadeSettledPrivateFillBundle::abi_decode(&settlement_bundle.data)
                        .map_err(IndexerError::parse)
                        .map(SettlementBundleData::RenegadeSettledPrivateFill)
                }
            },
            _ => Err(IndexerError::invalid_settlement_bundle(format!(
                "invalid settlement bundle type: {bundle_type}"
            ))),
        }
    }
}

impl SettlementBundleData {
    /// Get the input balance nullifier from the settlement bundle data, if one
    /// was spent
    pub fn get_input_balance_nullifier(&self) -> Option<Nullifier> {
        let nullifier_u256 = match self {
            Self::RenegadeSettledIntentFirstFill(bundle) => {
                bundle.auth.statement.oldBalanceNullifier
            },
            Self::RenegadeSettledIntent(bundle) => bundle.auth.statement.oldBalanceNullifier,
            Self::RenegadeSettledPrivateFirstFill(bundle) => {
                bundle.auth.statement.oldBalanceNullifier
            },
            Self::RenegadeSettledPrivateFill(bundle) => bundle.auth.statement.oldBalanceNullifier,
            // Natively-settled bundles don't spend a balance state object's nullifier
            _ => return None,
        };

        Some(u256_to_scalar(&nullifier_u256))
    }

    /// Get the data required to create a new output balance from a public fill
    /// using this settlement bundle data, if it is a renegade-settled,
    /// public-fill intent first fill bundle
    pub fn get_new_output_balance_from_public_fill_data(
        &self,
    ) -> Result<
        Option<(PreMatchBalanceShare, PostMatchBalanceShare, FixedPoint, Scalar)>,
        IndexerError,
    > {
        if !matches!(self, Self::RenegadeSettledIntentFirstFill(_)) {
            return Ok(None);
        }

        let output_balance_bundle_data = self.get_output_balance_bundle_data()?.unwrap();

        if !matches!(output_balance_bundle_data, OutputBalanceBundleData::NewBalanceBundle(_)) {
            return Ok(None);
        }

        let pre_match_balance_share =
            output_balance_bundle_data.get_pre_match_balance_shares().unwrap();

        let post_match_balance_share =
            self.get_pre_update_balance_shares(false /* is_input_balance */).unwrap();

        let relayer_fee_rate = self.get_relayer_fee_rate().unwrap();

        let recovery_id = output_balance_bundle_data.get_balance_recovery_id();

        Ok(Some((pre_match_balance_share, post_match_balance_share, relayer_fee_rate, recovery_id)))
    }

    /// Get the relayer fee rate from the settlement bundle data, if it was a
    /// publicly-settled bundle
    pub fn get_relayer_fee_rate(&self) -> Option<FixedPoint> {
        match self {
            Self::PublicIntentPublicBalance(bundle) => {
                Some(bundle.relayerFeeRate.rate.clone().into())
            },
            Self::RenegadeSettledIntentFirstFill(bundle) => {
                Some(bundle.settlementStatement.relayerFee.clone().into())
            },
            Self::RenegadeSettledIntent(bundle) => {
                Some(bundle.settlementStatement.relayerFee.clone().into())
            },
            _ => None,
        }
    }

    /// Get the output balance bundle data from the settlement bundle data, if
    /// it was a renegade-settled bundle
    pub fn get_output_balance_bundle_data(
        &self,
    ) -> Result<Option<OutputBalanceBundleData>, IndexerError> {
        let output_balance_bundle = match self {
            Self::RenegadeSettledIntentFirstFill(bundle) => &bundle.outputBalanceBundle,
            Self::RenegadeSettledIntent(bundle) => &bundle.outputBalanceBundle,
            Self::RenegadeSettledPrivateFirstFill(_) => todo!(),
            Self::RenegadeSettledPrivateFill(_) => todo!(),
            // Natively-settled bundles don't update a private output balance
            _ => return Ok(None),
        };

        let output_balance_bundle_data = output_balance_bundle.try_into()?;

        Ok(Some(output_balance_bundle_data))
    }
    /// Get the output balance nullifier from the settlement bundle data, if one
    /// was spent
    pub fn get_output_balance_nullifier(&self) -> Result<Option<Nullifier>, IndexerError> {
        let output_balance_bundle_data = self.get_output_balance_bundle_data()?;
        let maybe_output_balance_nullifier =
            output_balance_bundle_data.and_then(|bundle| bundle.get_balance_nullifier());

        Ok(maybe_output_balance_nullifier)
    }

    /// Get the intent nullifier from the settlement bundle data, if one was
    /// spent
    pub fn get_intent_nullifier(&self) -> Option<Nullifier> {
        let nullifier_u256 = match self {
            Self::PrivateIntentPublicBalance(bundle) => bundle.auth.statement.oldIntentNullifier,
            Self::RenegadeSettledIntent(bundle) => bundle.auth.statement.oldIntentNullifier,
            Self::RenegadeSettledPrivateFill(bundle) => bundle.auth.statement.oldIntentNullifier,
            // Public-intent & first-fill bundles don't spend an intent nullifier
            _ => return None,
        };

        Some(u256_to_scalar(&nullifier_u256))
    }

    /// Get the recovery ID of the intent from the settlement bundle data, if
    /// any
    pub fn get_intent_recovery_id(&self) -> Option<Scalar> {
        let recovery_id_u256 = match self {
            Self::PrivateIntentPublicBalanceFirstFill(bundle) => bundle.auth.statement.recoveryId,
            Self::PrivateIntentPublicBalance(bundle) => bundle.auth.statement.recoveryId,
            Self::RenegadeSettledIntentFirstFill(bundle) => bundle.auth.statement.intentRecoveryId,
            Self::RenegadeSettledIntent(bundle) => bundle.auth.statement.intentRecoveryId,
            Self::RenegadeSettledPrivateFirstFill(bundle) => bundle.auth.statement.intentRecoveryId,
            Self::RenegadeSettledPrivateFill(bundle) => bundle.auth.statement.intentRecoveryId,
            // Public-intent bundles don't contain an intent recovery ID
            _ => return None,
        };

        Some(u256_to_scalar(&recovery_id_u256))
    }

    /// Get the public intent hash from the settlement bundle data, if any
    pub fn get_public_intent_hash(&self) -> Option<B256> {
        match self {
            Self::PublicIntentPublicBalance(bundle) => {
                Some(keccak256(bundle.auth.permit.abi_encode()))
            },
            // Private-intent bundles don't contain a public intent hash
            _ => None,
        }
    }

    /// Get the pre-update input/output balance public shares from the
    /// settlement bundle data, if it was a Renegade-settled, public-fill
    /// bundle
    pub fn get_pre_update_balance_shares(
        &self,
        is_input_balance: bool,
    ) -> Option<PostMatchBalanceShare> {
        let shares = match self {
            Self::RenegadeSettledIntentFirstFill(bundle) => {
                if is_input_balance {
                    bundle.settlementStatement.inBalancePublicShares.clone()
                } else {
                    bundle.settlementStatement.outBalancePublicShares.clone()
                }
            },
            Self::RenegadeSettledIntent(bundle) => {
                if is_input_balance {
                    bundle.settlementStatement.inBalancePublicShares.clone()
                } else {
                    bundle.settlementStatement.outBalancePublicShares.clone()
                }
            },
            // Natively-settled / private-fill bundles don't leak pre-update balance public shares
            _ => return None,
        };

        Some(shares.into())
    }

    /// Try to decode the public intent with the given hash from the given
    /// settlement bundle.
    ///
    /// Returns `None` if the settlement bundle doesn't contain the public
    /// intent.
    pub fn try_decode_public_intent(
        &self,
        intent_hash: B256,
    ) -> Result<Option<Intent>, IndexerError> {
        let public_intent_hash = self.get_public_intent_hash();

        if public_intent_hash != Some(intent_hash) {
            return Ok(None);
        }

        let maybe_intent = self.get_public_intent();

        Ok(maybe_intent)
    }

    /// Get the public intent from the settlement bundle data, if any
    fn get_public_intent(&self) -> Option<Intent> {
        match self {
            Self::PublicIntentPublicBalance(bundle) => {
                let sol_intent = &bundle.auth.permit.intent;

                let min_price = FixedPoint::from_repr(u256_to_scalar(&sol_intent.minPrice.repr));
                let amount_in = u256_to_amount(sol_intent.amountIn);

                Some(Intent {
                    in_token: sol_intent.inToken,
                    out_token: sol_intent.outToken,
                    owner: sol_intent.owner,
                    min_price,
                    amount_in,
                })
            },
            // Private-intent bundles don't contain a public intent
            _ => None,
        }
    }
}
