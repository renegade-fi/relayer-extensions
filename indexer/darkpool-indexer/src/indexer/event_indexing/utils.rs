//! Helper functions used for event indexing

use std::iter;

use alloy::primitives::B256;
use renegade_circuit_types::{
    Nullifier,
    fixed_point::FixedPointShare,
    intent::{Intent, IntentShare, PreMatchIntentShare},
    traits::BaseType,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    IntentPreMatchShare, ObligationBundle, SettlementBundle,
};

use crate::{
    darkpool_client::DarkpoolClient,
    indexer::{
        error::IndexerError,
        event_indexing::types::{
            obligation_bundle::ObligationBundleData, settlement_bundle::SettlementBundleData,
        },
    },
    state_transitions::{
        create_balance::BalanceCreationData, create_intent::IntentCreationData,
        settle_match_into_balance::BalanceSettlementData,
        settle_match_into_intent::IntentSettlementData,
    },
    types::ObligationAmounts,
};

/// Try to decode the balance creation data for the given party's newly-created
/// output balance from the given settlement bundle.
///
/// Returns `None` if the settlement bundle does not contain a newly-created
/// output balance with a matching recovery ID.
pub fn try_decode_new_output_balance_creation_data(
    recovery_id: Scalar,
    settlement_bundle: &SettlementBundle,
) -> Result<Option<BalanceCreationData>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let maybe_output_balance_bundle_data =
        settlement_bundle_data.get_output_balance_bundle_data()?;

    if maybe_output_balance_bundle_data.is_none() {
        return Ok(None);
    }

    let output_balance_bundle_data = maybe_output_balance_bundle_data.unwrap();
    let balance_recovery_id = output_balance_bundle_data.get_balance_recovery_id();

    if balance_recovery_id != recovery_id {
        return Ok(None);
    }

    let maybe_pre_match_balance_share = output_balance_bundle_data.get_pre_match_balance_shares();

    let maybe_post_match_balance_share =
        settlement_bundle_data.get_pre_update_balance_shares(false /* is_input_balance */);

    if maybe_pre_match_balance_share.is_none() || maybe_post_match_balance_share.is_none() {
        return Ok(None);
    }

    let pre_match_balance_share = maybe_pre_match_balance_share.unwrap();
    let post_match_balance_share = maybe_post_match_balance_share.unwrap();

    let balance_creation_data =
        BalanceCreationData::NewOutputBalance { pre_match_balance_share, post_match_balance_share };

    Ok(Some(balance_creation_data))
}

/// Try to decode the balance settlement data from the match party's
/// settlement bundle & obligation bundle.
///
/// Returns `None` if the spent nullifier does not match the party's input or
/// output balance nullifier.
pub async fn try_decode_balance_settlement_data(
    darkpool_client: &DarkpoolClient,
    block_number: u64,
    nullifier: Nullifier,
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<Option<BalanceSettlementData>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let obligation_bundle_data: ObligationBundleData = obligation_bundle.try_into()?;

    let in_balance_nullifier = settlement_bundle_data.get_input_balance_nullifier();
    let out_balance_nullifier = settlement_bundle_data.get_output_balance_nullifier()?;

    if in_balance_nullifier == Some(nullifier) {
        try_get_input_balance_settlement_data(
            &settlement_bundle_data,
            &obligation_bundle_data,
            is_party0,
        )
    } else if out_balance_nullifier == Some(nullifier) {
        try_get_output_balance_settlement_data(
            darkpool_client,
            block_number,
            &settlement_bundle_data,
            &obligation_bundle_data,
            is_party0,
        )
        .await
    } else {
        // The spent nullifier corresponds neither to the input nor output balance
        // nullifier
        return Ok(None);
    }
}

/// Get the balance settlement data associated with the given party's settlement
/// bundle and the obligation bundle, assuming the balance is the party's input
/// balance.
fn try_get_input_balance_settlement_data(
    settlement_bundle_data: &SettlementBundleData,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
) -> Result<Option<BalanceSettlementData>, IndexerError> {
    match settlement_bundle_data {
        SettlementBundleData::RenegadeSettledIntentFirstFill(_)
        | SettlementBundleData::RenegadeSettledIntent(_) => {
            let settlement_obligation = obligation_bundle_data
                .get_public_settlement_obligation(is_party0)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected public obligation bundle",
                ))?
                .into();

            Ok(Some(BalanceSettlementData::PublicFillInputBalance { settlement_obligation }))
        },
        SettlementBundleData::RenegadeSettledPrivateFirstFill(_)
        | SettlementBundleData::RenegadeSettledPrivateFill(_) => {
            let updated_balance_shares = obligation_bundle_data
                .get_balance_shares_in_private_match(is_party0, true /* is_input_balance */)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected private obligation bundle",
                ))?;

            Ok(Some(BalanceSettlementData::PrivateFill(updated_balance_shares)))
        },
        // Natively-settled bundles don't update any balance state objects
        _ => Ok(None),
    }
}

/// Get the balance settlement data associated with the given party's settlement
/// bundle and the obligation bundle, assuming the balance is the party's output
/// balance.
async fn try_get_output_balance_settlement_data(
    darkpool_client: &DarkpoolClient,
    block_number: u64,
    settlement_bundle_data: &SettlementBundleData,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
) -> Result<Option<BalanceSettlementData>, IndexerError> {
    let relayer_fee_rate = match settlement_bundle_data {
        SettlementBundleData::RenegadeSettledIntentFirstFill(bundle) => {
            Some(bundle.settlementStatement.relayerFee.clone().into())
        },
        SettlementBundleData::RenegadeSettledIntent(bundle) => {
            Some(bundle.settlementStatement.relayerFee.clone().into())
        },
        _ => None,
    };

    match settlement_bundle_data {
        SettlementBundleData::RenegadeSettledIntentFirstFill(_)
        | SettlementBundleData::RenegadeSettledIntent(_) => {
            let settlement_obligation = obligation_bundle_data
                .get_public_settlement_obligation(is_party0)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected public obligation bundle",
                ))?
                .into();

            let relayer_fee_rate = relayer_fee_rate.unwrap();

            let (asset0, asset1) =
                obligation_bundle_data.get_public_obligation_trading_pair().ok_or(
                    IndexerError::invalid_obligation_bundle("expected public obligation bundle"),
                )?;

            let protocol_fee_rate = darkpool_client
                .get_protocol_fee_rate_at_block(asset0, asset1, block_number)
                .await
                .map_err(IndexerError::rpc)?;

            Ok(Some(BalanceSettlementData::PublicFillOutputBalance {
                settlement_obligation,
                relayer_fee_rate,
                protocol_fee_rate,
            }))
        },
        SettlementBundleData::RenegadeSettledPrivateFirstFill(_)
        | SettlementBundleData::RenegadeSettledPrivateFill(_) => {
            let updated_balance_shares = obligation_bundle_data
                .get_balance_shares_in_private_match(is_party0, false /* is_input_balance */)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected private obligation bundle",
                ))?;

            Ok(Some(BalanceSettlementData::PrivateFill(updated_balance_shares)))
        },
        // Natively-settled bundles don't update any balance state objects
        _ => Ok(None),
    }
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

            let settlement_obligation = obligation_bundle_data
                .get_public_settlement_obligation(is_party0)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected public obligation bundle",
                ))?
                .into();

            Ok(Some(IntentCreationData::PublicFill {
                pre_match_full_intent_share,
                settlement_obligation,
            }))
        },
        SettlementBundleData::RenegadeSettledIntentFirstFill(bundle) => {
            // The `intentPublicShare` field in the auth statement excludes the public share
            // of the intent amount
            let pre_match_intent_share = bundle.auth.statement.intentPublicShare.clone().into();

            let amount_in = u256_to_scalar(&bundle.settlementStatement.amountPublicShare);

            let pre_match_full_intent_share =
                from_pre_match_intent_and_amount(pre_match_intent_share, amount_in);

            let settlement_obligation = obligation_bundle_data
                .get_public_settlement_obligation(is_party0)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected public obligation bundle",
                ))?
                .into();

            Ok(Some(IntentCreationData::PublicFill {
                pre_match_full_intent_share,
                settlement_obligation,
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

/// Construct a circuit `IntentShare` from a `PreMatchIntentShare` and an amount
fn from_pre_match_intent_and_amount(
    pre_match_intent_share: PreMatchIntentShare,
    amount_in: Scalar,
) -> IntentShare {
    let PreMatchIntentShare { in_token, out_token, owner, min_price } = pre_match_intent_share;

    IntentShare { in_token, out_token, owner, min_price, amount_in }
}

/// Try to decode the public intent data (the intent, and the
/// obligation input amount) from the given party's settlement & obligation
/// bundles.
pub fn try_decode_public_intent_data(
    intent_hash: B256,
    settlement_bundle: &SettlementBundle,
    obligation_bundle_data: &ObligationBundleData,
    is_party0: bool,
) -> Result<Option<(Intent, Scalar)>, IndexerError> {
    let settlement_bundle_data: SettlementBundleData = settlement_bundle.try_into()?;
    let maybe_intent = settlement_bundle_data.try_decode_public_intent(intent_hash)?;
    let ObligationAmounts { amount_in, .. } = obligation_bundle_data
        .get_public_obligation_amounts(is_party0)
        .ok_or(IndexerError::invalid_obligation_bundle("expected public obligation bundle"))?;

    Ok(maybe_intent.map(|intent| (intent, amount_in)))
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
    nullifier: Nullifier,
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
        SettlementBundleData::RenegadeSettledIntent(_) => {
            let settlement_obligation = obligation_bundle_data
                .get_public_settlement_obligation(is_party0)
                .ok_or(IndexerError::invalid_obligation_bundle(
                    "expected public obligation bundle",
                ))?
                .into();

            Ok(Some(IntentSettlementData::RenegadeSettledPublicFill { settlement_obligation }))
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
