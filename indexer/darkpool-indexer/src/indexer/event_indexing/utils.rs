//! Helper functions used for event indexing

use std::iter;

use renegade_circuit_types::{
    balance::PostMatchBalanceShare,
    fixed_point::FixedPointShare,
    intent::{IntentShare, PreMatchIntentShare},
    traits::BaseType,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    IntentPreMatchShare, ObligationBundle, PostMatchBalanceShare as ContractPostMatchBalanceShare,
    SettlementBundle,
};

use crate::{
    indexer::{
        error::IndexerError,
        event_indexing::types::{
            obligation_bundle::ObligationBundleData, settlement_bundle::SettlementBundleData,
        },
    },
    state_transitions::{
        create_intent::{IntentCreationData, from_pre_match_intent_and_amount},
        settle_match_into_balance::BalanceSettlementData,
        settle_match_into_intent::IntentSettlementData,
    },
    types::ObligationAmounts,
};

/// Try to decode the balance settlement data from the match party's
/// settlement bundle & obligation bundle.
///
/// Returns `None` if the spent nullifier does not match the party's input or
/// output balance nullifier.
pub fn try_decode_balance_settlement_data(
    nullifier: Scalar,
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<Option<BalanceSettlementData>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let obligation_bundle_data: ObligationBundleData = obligation_bundle.try_into()?;

    let in_balance_nullifier =
        settlement_bundle_data.get_balance_nullifier(true /* is_input_balance */);

    let out_balance_nullifier =
        settlement_bundle_data.get_balance_nullifier(false /* is_input_balance */);

    if in_balance_nullifier == Some(nullifier) {
        get_balance_settlement_data(
            &settlement_bundle_data,
            &obligation_bundle_data,
            is_party0,
            true, // is_input_balance
        )
    } else if out_balance_nullifier == Some(nullifier) {
        get_balance_settlement_data(
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

/// Get the balance settlement data associated with this settlement & obligation
/// bundle (if any)
fn get_balance_settlement_data(
    settlement_bundle_data: &SettlementBundleData,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
    is_input_balance: bool,
) -> Result<Option<BalanceSettlementData>, IndexerError> {
    match settlement_bundle_data {
        // For public-fill bundles, we parse the pre-update balance public shares & replicate the
        // contract logic for updating them
        SettlementBundleData::RenegadeSettledIntentFirstFill(_)
        | SettlementBundleData::RenegadeSettledIntent(_) => {
            let pre_update_balance_shares =
                settlement_bundle_data.get_pre_update_balance_shares(is_input_balance).unwrap(); // It's safe to unwrap in this match arm

            let obligation_amounts =
                obligation_bundle_data.get_public_obligation_amounts(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
                )?;

            Ok(Some(BalanceSettlementData::PublicFill {
                pre_update_balance_shares,
                obligation_amounts,
                is_input_balance,
            }))
        },
        // For private-fill bundles, we parse the updated balance public shares directly from the
        // obligation bundle data
        SettlementBundleData::RenegadeSettledPrivateFirstFill(_)
        | SettlementBundleData::RenegadeSettledPrivateFill(_) => {
            let updated_balance_shares = obligation_bundle_data
                .get_balance_shares_in_private_match(is_party0, is_input_balance)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected private obligation bundle",
                ))?;

            Ok(Some(BalanceSettlementData::PrivateFill(updated_balance_shares)))
        },
        // Natively-settled bundles don't update any balance state objects
        _ => Ok(None),
    }
}

/// Convert a contract `PostMatchBalanceShare` to a circuit
/// `PostMatchBalanceShare`
pub fn to_circuit_post_match_balance_share(
    post_match_balance_share: &ContractPostMatchBalanceShare,
) -> PostMatchBalanceShare {
    let ContractPostMatchBalanceShare { relayerFeeBalance, protocolFeeBalance, amount } =
        post_match_balance_share.clone();

    let relayer_fee_balance = u256_to_scalar(&relayerFeeBalance);
    let protocol_fee_balance = u256_to_scalar(&protocolFeeBalance);
    let amount = u256_to_scalar(&amount);

    PostMatchBalanceShare { relayer_fee_balance, protocol_fee_balance, amount }
}

/// Try to decode the intent creation data for the given party's newly-created
/// intent from the given settlement & obligation bundles.
///
/// Returns `None` if the settlement bundle does not contain a newly-created
/// intent with a matching recovery ID.
pub fn try_decode_intent_creation_data(
    recovery_id: Scalar,
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<Option<IntentCreationData>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let obligation_bundle_data: ObligationBundleData = obligation_bundle.try_into()?;

    let intent_recovery_id = settlement_bundle_data.get_intent_recovery_id();

    if intent_recovery_id != Some(recovery_id) {
        return Ok(None);
    }

    get_intent_creation_data(&settlement_bundle_data, &obligation_bundle_data, is_party0)
}

/// Get the intent creation data for the given party's newly-created intent, if
/// this was a first-fill bundle
fn get_intent_creation_data(
    settlement_bundle_data: &SettlementBundleData,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
) -> Result<Option<IntentCreationData>, IndexerError> {
    match settlement_bundle_data {
        SettlementBundleData::PrivateIntentPublicBalanceFirstFill(bundle) => {
            let pre_match_full_intent_share: IntentShare =
                bundle.auth.statement.intentPublicShare.clone().into();

            let ObligationAmounts { amount_in, .. } =
                obligation_bundle_data.get_public_obligation_amounts(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
                )?;

            Ok(Some(IntentCreationData::NativelySettledPrivateIntent {
                pre_match_full_intent_share,
                amount_in,
            }))
        },
        SettlementBundleData::RenegadeSettledIntentFirstFill(bundle) => {
            // The `intentPublicShare` field in the auth statement excludes the public share
            // of the intent amount
            let pre_match_intent_shares =
                to_circuit_pre_match_intent_share(&bundle.auth.statement.intentPublicShare);

            let pre_match_amount_share =
                u256_to_scalar(&bundle.settlementStatement.amountPublicShare);

            let ObligationAmounts { amount_in, .. } =
                obligation_bundle_data.get_public_obligation_amounts(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
                )?;

            Ok(Some(IntentCreationData::RenegadeSettledPublicFill {
                pre_match_partial_intent_share: pre_match_intent_shares,
                pre_match_amount_share,
                amount_in,
            }))
        },
        SettlementBundleData::RenegadeSettledPrivateFirstFill(bundle) => {
            // The `intentPublicShare` field in the auth statement excludes the public share
            // of the intent amount
            let pre_match_intent_share =
                to_circuit_pre_match_intent_share(&bundle.auth.statement.intentPublicShare);

            let amount_public_share =
                obligation_bundle_data.get_updated_intent_amount_public_share(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected private obligation bundle"),
                )?;

            let updated_intent_share =
                from_pre_match_intent_and_amount(pre_match_intent_share, amount_public_share);

            Ok(Some(IntentCreationData::RenegadeSettledPrivateFill(updated_intent_share)))
        },
        // Non-first-fill bundles don't create a new intent
        _ => Ok(None),
    }
}

/// Convert a contract `IntentPreMatchShare` to a circuit
/// `PreMatchIntentShare`
fn to_circuit_pre_match_intent_share(
    contract_pre_match_share: &IntentPreMatchShare,
) -> PreMatchIntentShare {
    let IntentPreMatchShare { inToken, outToken, owner, minPrice } = contract_pre_match_share;

    let in_token = u256_to_scalar(inToken);
    let out_token = u256_to_scalar(outToken);
    let owner = u256_to_scalar(owner);

    let min_price = FixedPointShare::from_scalars(&mut iter::once(u256_to_scalar(minPrice)));

    PreMatchIntentShare { in_token, out_token, owner, min_price }
}

/// Try to decode the intent settlement data for the given party from the given
/// settlement & obligation bundles.
///
/// Returns `None` if the settlement bundle does not contain an updated intent
/// with a matching nullifier.
pub fn try_decode_intent_settlement_data(
    nullifier: Scalar,
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<Option<IntentSettlementData>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let obligation_bundle_data: ObligationBundleData = obligation_bundle.try_into()?;

    let intent_nullifier = settlement_bundle_data.get_intent_nullifier();

    if intent_nullifier != Some(nullifier) {
        return Ok(None);
    }

    get_intent_settlement_data(&settlement_bundle_data, &obligation_bundle_data, is_party0)
}

/// Get the intent settlement data for the given party, if this was a
/// non-first-fill bundle
fn get_intent_settlement_data(
    settlement_bundle_data: &SettlementBundleData,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
) -> Result<Option<IntentSettlementData>, IndexerError> {
    match settlement_bundle_data {
        SettlementBundleData::PrivateIntentPublicBalance(bundle) => {
            let updated_amount_share = u256_to_scalar(&bundle.auth.statement.newAmountShare);
            Ok(Some(IntentSettlementData::UpdatedAmountShare(updated_amount_share)))
        },
        SettlementBundleData::RenegadeSettledIntent(bundle) => {
            // The `amountPublicShare` field in the settlement statement is the pre-update
            // public share of the intent amount
            let pre_match_amount_share =
                u256_to_scalar(&bundle.settlementStatement.amountPublicShare);

            // We replicate the contract logic for updating the intent amount public share
            let ObligationAmounts { amount_in, .. } =
                obligation_bundle_data.get_public_obligation_amounts(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
                )?;

            Ok(Some(IntentSettlementData::RenegadeSettledPublicFill {
                pre_match_amount_share,
                amount_in,
            }))
        },
        SettlementBundleData::RenegadeSettledPrivateFill(_) => {
            let amount_public_share =
                obligation_bundle_data.get_updated_intent_amount_public_share(is_party0).ok_or(
                    IndexerError::invalid_obligation_bundle("expected private obligation bundle"),
                )?;

            Ok(Some(IntentSettlementData::UpdatedAmountShare(amount_public_share)))
        },
        // First-fill bundles don't create a new intent
        _ => Ok(None),
    }
}
