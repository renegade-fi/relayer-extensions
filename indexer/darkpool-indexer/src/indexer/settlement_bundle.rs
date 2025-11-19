//! Defines a wrapper type & parsing utilities for the various kinds of
//! settlement bundles

use alloy::{
    primitives::{B256, U256, keccak256},
    sol_types::SolValue,
};
use renegade_circuit_types::{fixed_point::FixedPoint, intent::Intent};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateIntentPublicBalanceBundle, PrivateIntentPublicBalanceFirstFillBundle,
    PrivateObligationBundle, PublicIntentPublicBalanceBundle, RenegadeSettledIntentBundle,
    RenegadeSettledIntentFirstFillBundle, RenegadeSettledPrivateFillBundle,
    RenegadeSettledPrivateFirstFillBundle, SettlementBundle,
};

use crate::{darkpool_client::utils::u256_to_amount, indexer::error::IndexerError};

// -------------
// | Constants |
// -------------

/// The value for the `NATIVELY_SETTLED_PUBLIC_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const NATIVELY_SETTLED_PUBLIC_INTENT: u8 = 0;
/// The value for the `NATIVELY_SETTLED_PRIVATE_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const NATIVELY_SETTLED_PRIVATE_INTENT: u8 = 1;
/// The value for the `RENEGADE_SETTLED_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const RENEGADE_SETTLED_INTENT: u8 = 2;
/// The value for the `RENEGADE_SETTLED_PRIVATE_FILL` variant of the Solidity
/// `SettlementBundleType` enum
const RENEGADE_SETTLED_PRIVATE_FILL: u8 = 3;

// ---------
// | Types |
// ---------

/// A wrapper around the different types of settlement bundle data
enum SettlementBundleData {
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

impl SettlementBundleData {
    /// Get the balance nullifier from the settlement bundle data, if one was
    /// spent
    // TODO: Mux between inBalance / outBalance nullifier once the ABI is finalized
    pub fn get_balance_nullifier(&self, _is_input_balance: bool) -> Option<Scalar> {
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

    /// Get the public shares for the new relayer fee, protocol fee, and amount
    /// in the private balance associated with this settlement bundle (if any).
    ///
    /// In the case of private-fill bundles, we parse the updated shares from
    /// the obligation bundle data.
    pub fn get_new_balance_public_shares(
        &self,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
        is_input_balance: bool,
    ) -> Result<Option<(Scalar, Scalar, Scalar)>, IndexerError> {
        let [
            new_relayer_fee_public_share_u256,
            new_protocol_fee_public_share_u256,
            new_amount_public_share_u256,
        ] = match self {
            Self::RenegadeSettledIntentFirstFill(bundle) => {
                if is_input_balance {
                    bundle.settlementStatement.inBalancePublicShares
                } else {
                    bundle.settlementStatement.outBalancePublicShares
                }
            },
            Self::RenegadeSettledIntent(bundle) => {
                if is_input_balance {
                    bundle.settlementStatement.inBalancePublicShares
                } else {
                    bundle.settlementStatement.outBalancePublicShares
                }
            },
            Self::RenegadeSettledPrivateFirstFill(_) | Self::RenegadeSettledPrivateFill(_) => {
                decode_balance_shares_from_private_obligation_bundle(
                    obligation_bundle,
                    is_party0,
                    is_input_balance,
                )?
            },
            // Natively-settled bundles don't update any balance state objects
            _ => return Ok(None),
        };

        Ok(Some((
            u256_to_scalar(&new_relayer_fee_public_share_u256),
            u256_to_scalar(&new_protocol_fee_public_share_u256),
            u256_to_scalar(&new_amount_public_share_u256),
        )))
    }

    /// Get the public intent from the settlement bundle data, if any
    pub fn get_public_intent(&self) -> Option<Intent> {
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

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Try to decode the new balance public shares from the match party's
/// settlement bundle & obligation bundle.
///
/// Returns `None` if the spent nullifier does not match the party's input or
/// output balance nullifier.
pub fn try_decode_balance_shares_for_party(
    nullifier: Scalar,
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<Option<(Scalar, Scalar, Scalar)>, IndexerError> {
    let settlement_bundle_data = decode_settlement_bundle_data(settlement_bundle)?;

    let in_balance_nullifier =
        settlement_bundle_data.get_balance_nullifier(true /* is_input_balance */);

    let out_balance_nullifier =
        settlement_bundle_data.get_balance_nullifier(false /* is_input_balance */);

    if in_balance_nullifier == Some(nullifier) {
        settlement_bundle_data.get_new_balance_public_shares(
            obligation_bundle,
            is_party0,
            true, // is_input_balance
        )
    } else if out_balance_nullifier == Some(nullifier) {
        settlement_bundle_data.get_new_balance_public_shares(
            obligation_bundle,
            is_party0,
            false, // is_input_balance
        )
    } else {
        // The spent nullifier corresponds neither to the input nor output balance
        // nullifier
        return Ok(None);
    }
}

/// Try to decode the public intent with the given hash from the given
/// settlement bundle.
///
/// Returns `None` if the settlement bundle doesn't contain the public intent.
pub fn try_decode_public_intent(
    intent_hash: B256,
    settlement_bundle: &SettlementBundle,
) -> Result<Option<Intent>, IndexerError> {
    let settlement_bundle_data = decode_settlement_bundle_data(settlement_bundle)?;

    let public_intent_hash = settlement_bundle_data.get_public_intent_hash();

    if public_intent_hash != Some(intent_hash) {
        return Ok(None);
    }

    let maybe_intent = settlement_bundle_data.get_public_intent();

    Ok(maybe_intent)
}

/// Decode the settlement bundle data for a renegade-settled bundle.
///
/// Returns `None` if the bundle is a natively-settled bundle.
fn decode_settlement_bundle_data(
    settlement_bundle: &SettlementBundle,
) -> Result<SettlementBundleData, IndexerError> {
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

/// Decode the given party's new balance public shares from the given obligation
/// bundle, assuming it is a private obligation bundle.
fn decode_balance_shares_from_private_obligation_bundle(
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
    is_input_balance: bool,
) -> Result<[U256; 3], IndexerError> {
    let private_obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
        .map_err(IndexerError::parse)?;

    match (is_party0, is_input_balance) {
        (true, true) => Ok(private_obligation_bundle.statement.newInBalancePublicShares0),
        (true, false) => Ok(private_obligation_bundle.statement.newOutBalancePublicShares0),
        (false, true) => Ok(private_obligation_bundle.statement.newInBalancePublicShares1),
        (false, false) => Ok(private_obligation_bundle.statement.newOutBalancePublicShares1),
    }
}
