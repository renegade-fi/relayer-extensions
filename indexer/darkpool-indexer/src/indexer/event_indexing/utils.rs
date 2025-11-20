//! Helper functions used for event indexing

use std::iter;

use renegade_circuit_types::{intent::IntentShare, traits::BaseType};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{ObligationBundle, SettlementBundle};

use crate::indexer::{
    error::IndexerError,
    event_indexing::types::{
        obligation_bundle::ObligationBundleData, settlement_bundle::SettlementBundleData,
    },
};

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
) -> Result<Option<[Scalar; 3]>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let obligation_bundle_data: ObligationBundleData = obligation_bundle.try_into()?;

    let in_balance_nullifier =
        settlement_bundle_data.get_balance_nullifier(true /* is_input_balance */);

    let out_balance_nullifier =
        settlement_bundle_data.get_balance_nullifier(false /* is_input_balance */);

    if in_balance_nullifier == Some(nullifier) {
        get_updated_balance_public_shares(
            &settlement_bundle_data,
            &obligation_bundle_data,
            is_party0,
            true, // is_input_balance
        )
    } else if out_balance_nullifier == Some(nullifier) {
        get_updated_balance_public_shares(
            &settlement_bundle_data,
            &obligation_bundle_data,
            is_party0,
            false, // is_input_balance
        )
    } else {
        // The spent nullifier corresponds neither to the input nor output balance
        // nullifier
        return Ok(None);
    }
}

/// Get the public shares for the new relayer fee, protocol fee, and amount
/// in the private balance associated with this settlement bundle (if any).
///
/// In the case of private-fill bundles, we parse the updated shares from
/// the obligation bundle data.
fn get_updated_balance_public_shares(
    settlement_bundle_data: &SettlementBundleData,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
    is_input_balance: bool,
) -> Result<Option<[Scalar; 3]>, IndexerError> {
    let [
        new_relayer_fee_public_share_u256,
        new_protocol_fee_public_share_u256,
        new_amount_public_share_u256,
    ] = match settlement_bundle_data {
        // For public-fill bundles, we parse the pre-update balance public shares & replicate the
        // contract logic for updating them
        SettlementBundleData::RenegadeSettledIntentFirstFill(_)
        | SettlementBundleData::RenegadeSettledIntent(_) => {
            let [
                new_relayer_fee_public_share_u256,
                new_protocol_fee_public_share_u256,
                mut new_amount_public_share_u256,
            ] = settlement_bundle_data
                .get_pre_update_balance_public_shares(is_input_balance)
                .unwrap(); // It's safe to unwrap in this match arm

            // TODO: Account for fee amounts
            if is_input_balance {
                let amount_in = obligation_bundle_data.get_amount_in(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
                )?;
                new_amount_public_share_u256 -= amount_in
            } else {
                let amount_out = obligation_bundle_data.get_amount_out(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
                )?;
                new_amount_public_share_u256 += amount_out;
            }

            [
                new_relayer_fee_public_share_u256,
                new_protocol_fee_public_share_u256,
                new_amount_public_share_u256,
            ]
        },
        // For private-fill bundles, we parse the updated balance public shares directly from the
        // obligation bundle data
        SettlementBundleData::RenegadeSettledPrivateFirstFill(_)
        | SettlementBundleData::RenegadeSettledPrivateFill(_) => obligation_bundle_data
            .get_updated_balance_public_shares(is_party0, is_input_balance)
            .ok_or(IndexerError::invalid_obligation_bundle("expected private obligation bundle"))?,
        // Natively-settled bundles don't update any balance state objects
        _ => return Ok(None),
    };

    Ok(Some([
        u256_to_scalar(&new_relayer_fee_public_share_u256),
        u256_to_scalar(&new_protocol_fee_public_share_u256),
        u256_to_scalar(&new_amount_public_share_u256),
    ]))
}

/// Try to decode the public shares for the given party's newly-created intent
/// from the given settlement & obligation bundles.
///
/// Returns `None` if the settlement bundle does not contain a newly-created
/// intent with a matching recovery ID.
pub fn try_decode_new_intent_shares_for_party(
    recovery_id: Scalar,
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<Option<IntentShare>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let obligation_bundle_data: ObligationBundleData = obligation_bundle.try_into()?;

    let intent_recovery_id = settlement_bundle_data.get_intent_recovery_id();

    if intent_recovery_id != Some(recovery_id) {
        return Ok(None);
    }

    get_new_intent_public_shares(&settlement_bundle_data, &obligation_bundle_data, is_party0)
}

/// Get the public shares for the given party's newly-created intent, if
/// this was a first-fill bundle
fn get_new_intent_public_shares(
    settlement_bundle_data: &SettlementBundleData,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
) -> Result<Option<IntentShare>, IndexerError> {
    match settlement_bundle_data {
        SettlementBundleData::PrivateIntentPublicBalanceFirstFill(bundle) => {
            let mut public_shares_iter =
                bundle.auth.statement.intentPublicShare.iter().map(u256_to_scalar);

            Ok(Some(IntentShare::from_scalars(&mut public_shares_iter)))
        },
        SettlementBundleData::RenegadeSettledIntentFirstFill(bundle) => {
            // The `intentPublicShare` field in the auth statement excludes the public share
            // of the intent amount
            let post_update_public_shares_u256_iter =
                bundle.auth.statement.intentPublicShare.iter();

            // We replicate the contract logic for updating the intent amount public share
            let pre_update_amount_public_share_u256 = bundle.settlementStatement.amountPublicShare;

            let amount_in = obligation_bundle_data.get_amount_in(is_party0).ok_or(
                IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
            )?;

            let post_update_amount_public_share_u256 =
                pre_update_amount_public_share_u256 - amount_in;

            let mut public_shares_iter = post_update_public_shares_u256_iter
                .chain(iter::once(&post_update_amount_public_share_u256))
                .map(u256_to_scalar);

            Ok(Some(IntentShare::from_scalars(&mut public_shares_iter)))
        },
        SettlementBundleData::RenegadeSettledPrivateFirstFill(bundle) => {
            // The `intentPublicShare` field in the auth statement excludes the public share
            // of the intent amount
            let public_shares_u256_iter = bundle.auth.statement.intentPublicShare.iter();

            let amount_public_share_u256 =
                obligation_bundle_data.get_updated_intent_amount_public_share(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected private obligation bundle"),
                )?;

            let mut public_shares_iter = public_shares_u256_iter
                .chain(iter::once(&amount_public_share_u256))
                .map(u256_to_scalar);

            Ok(Some(IntentShare::from_scalars(&mut public_shares_iter)))
        },
        // Non-first-fill bundles don't create a new intent
        _ => Ok(None),
    }
}
