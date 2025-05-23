//! Telemetry helpers for Arbitrum specific ABI functionality

use alloy_sol_types::SolCall;
use renegade_api::http::external_match::AtomicMatchApiBundle;
use renegade_circuit_types::wallet::Nullifier;
use renegade_constants::Scalar;
use renegade_darkpool_client::arbitrum::{
    abi::Darkpool::{processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall},
    contract_types::types::MatchPayload,
    helpers::deserialize_calldata,
};

use crate::{
    error::AuthServerError,
    server::{
        gas_sponsorship::contract_interaction::sponsorAtomicMatchSettleWithRefundOptionsCall,
        helpers::get_selector,
    },
};

/// Extracts the nullifier from a match bundle's settlement transaction
///
/// This function attempts to decode the settlement transaction data in two
/// ways:
/// 1. As a standard atomic match settle call
/// 2. As a match settle with receiver call
pub fn extract_nullifier_from_match_bundle(
    match_bundle: &AtomicMatchApiBundle,
) -> Result<Nullifier, AuthServerError> {
    let tx_data = match_bundle.settlement_tx.input.input().unwrap_or_default();
    let selector = get_selector(tx_data)?;

    // Retrieve serialized match payload from the transaction data
    let serialized_match_payload = match selector {
        processAtomicMatchSettleCall::SELECTOR => {
            processAtomicMatchSettleCall::abi_decode(tx_data)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        processAtomicMatchSettleWithReceiverCall::SELECTOR => {
            processAtomicMatchSettleWithReceiverCall::abi_decode(tx_data)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        sponsorAtomicMatchSettleWithRefundOptionsCall::SELECTOR => {
            sponsorAtomicMatchSettleWithRefundOptionsCall::abi_decode(tx_data)
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
