//! Helper methods for settlement processing
use alloy_sol_types::SolCall;
use contracts_common::types::MatchPayload;
use ethers::types::H256 as TxHash;
use renegade_api::http::external_match::AtomicMatchApiBundle;
use renegade_arbitrum_client::{
    abi::{processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall},
    client::ArbitrumClient,
    helpers::deserialize_calldata,
};
use renegade_circuit_types::{order::OrderSide, r#match::MatchResult, wallet::Nullifier};
use renegade_constants::Scalar;
use renegade_util::hex::biguint_to_hex_addr;
use std::time::Duration;

use crate::{
    error::AuthServerError,
    server::{
        handle_external_match::sponsorAtomicMatchSettleWithRefundOptionsCall, helpers::get_selector,
    },
};

// --- Constants --- //

/// The duration to await an atomic match settlement
pub const ATOMIC_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(30);

/// Await the result of the atomic match settlement to be submitted on-chain
///
/// Returns `true` if the settlement succeeded on-chain, `false` otherwise
pub(crate) async fn await_settlement(
    match_bundle: &AtomicMatchApiBundle,
    arbitrum_client: &ArbitrumClient,
) -> Result<bool, AuthServerError> {
    let nullifier = extract_nullifier_from_match_bundle(match_bundle)?;
    let tx_hash = arbitrum_client
        .await_nullifier_spent_from_selectors(
            nullifier,
            &[
                processAtomicMatchSettleCall::SELECTOR,
                processAtomicMatchSettleWithReceiverCall::SELECTOR,
            ],
            ATOMIC_SETTLEMENT_TIMEOUT,
        )
        .await
        .map_err(AuthServerError::arbitrum)?;

    verify_match_settlement_in_tx(arbitrum_client, match_bundle, tx_hash).await
}

/// Returns whether the provided tx corresponds to the provided external match
async fn verify_match_settlement_in_tx(
    arbitrum_client: &ArbitrumClient,
    match_bundle: &AtomicMatchApiBundle,
    tx: TxHash,
) -> Result<bool, AuthServerError> {
    let matches =
        arbitrum_client.find_external_matches_in_tx(tx).await.map_err(AuthServerError::arbitrum)?;
    let external_match = !matches.is_empty();

    if !external_match {
        return Ok(false);
    }

    let matching_settlement = matches.into_iter().any(|raw_match| match raw_match.try_into() {
        Ok(match_result) => is_matching_settlement(match_bundle, &match_result),
        Err(_) => false,
    });

    Ok(matching_settlement)
}

/// Returns `true` if the provided match bundle and match result are the same
fn is_matching_settlement(match_bundle: &AtomicMatchApiBundle, match_result: &MatchResult) -> bool {
    // Match result direction:
    // `true` (1) corresponds to the internal party selling the base
    // `false` (0) corresponds to the internal party buying the base
    let direction_matches = match match_result.direction {
        true => match_bundle.match_result.direction == OrderSide::Buy,
        false => match_bundle.match_result.direction == OrderSide::Sell,
    };

    let amounts_match = match_bundle.match_result.quote_amount == match_result.quote_amount
        && match_bundle.match_result.base_amount == match_result.base_amount;

    let tokens_match = match_bundle.match_result.quote_mint
        == biguint_to_hex_addr(&match_result.quote_mint)
        && match_bundle.match_result.base_mint == biguint_to_hex_addr(&match_result.base_mint);

    direction_matches && amounts_match && tokens_match
}

/// Extracts the nullifier from a match bundle's settlement transaction
///
/// This function attempts to decode the settlement transaction data in two
/// ways:
/// 1. As a standard atomic match settle call
/// 2. As a match settle with receiver call
fn extract_nullifier_from_match_bundle(
    match_bundle: &AtomicMatchApiBundle,
) -> Result<Nullifier, AuthServerError> {
    let tx_data = match_bundle
        .settlement_tx
        .data()
        .ok_or(AuthServerError::serde("No data in settlement tx"))?;

    let selector = get_selector(tx_data)?;

    // Retrieve serialized match payload from the transaction data
    let serialized_match_payload = match selector {
        processAtomicMatchSettleCall::SELECTOR => {
            processAtomicMatchSettleCall::abi_decode(tx_data, false)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        processAtomicMatchSettleWithReceiverCall::SELECTOR => {
            processAtomicMatchSettleWithReceiverCall::abi_decode(tx_data, false)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        sponsorAtomicMatchSettleWithRefundOptionsCall::SELECTOR => {
            sponsorAtomicMatchSettleWithRefundOptionsCall::abi_decode(tx_data, false)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        _ => {
            return Err(AuthServerError::serde("Invalid selector for settlement tx"));
        },
    };

    // Extract nullifier from the payload
    let match_payload = deserialize_calldata::<MatchPayload>(&serialized_match_payload)
        .map_err(AuthServerError::serde)?;
    let nullifier = Scalar::new(match_payload.valid_reblind_statement.original_shares_nullifier);

    Ok(nullifier)
}
