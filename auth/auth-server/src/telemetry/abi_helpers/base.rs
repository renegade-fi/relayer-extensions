//! Telemetry helpers for Base specific ABI functionality

use alloy_sol_types::SolCall;
use renegade_api::http::external_match::{AtomicMatchApiBundle, MalleableAtomicMatchApiBundle};
use renegade_circuit_types::wallet::Nullifier;
use renegade_darkpool_client::conversion::u256_to_scalar;
use renegade_solidity_abi::IDarkpool::{
    processAtomicMatchSettleCall, processMalleableAtomicMatchSettleCall,
    sponsorAtomicMatchSettleCall, sponsorMalleableAtomicMatchSettleCall,
};

use crate::{error::AuthServerError, server::helpers::get_selector};

/// Extract the nullifier from a match bundle
pub fn extract_nullifier_from_match_bundle(
    match_bundle: &AtomicMatchApiBundle,
) -> Result<Nullifier, AuthServerError> {
    let tx_data = match_bundle.settlement_tx.input.input().unwrap_or_default();
    extract_nullifier_from_settlement_tx_calldata(tx_data)
}

/// Extract the nullifier from a malleable match bundle
pub fn extract_nullifier_from_malleable_match_bundle(
    match_bundle: &MalleableAtomicMatchApiBundle,
) -> Result<Nullifier, AuthServerError> {
    let tx_data = match_bundle.settlement_tx.input.input().unwrap_or_default();
    extract_nullifier_from_settlement_tx_calldata(tx_data)
}

/// Extract a nullifier from settlement tx calldata
fn extract_nullifier_from_settlement_tx_calldata(
    tx_data: &[u8],
) -> Result<Nullifier, AuthServerError> {
    let selector = get_selector(tx_data)?;

    match selector {
        processAtomicMatchSettleCall::SELECTOR => {
            let call = processAtomicMatchSettleCall::abi_decode(tx_data)?;
            let nullifier = call.internalPartyPayload.validReblindStatement.originalSharesNullifier;
            Ok(u256_to_scalar(nullifier))
        },
        processMalleableAtomicMatchSettleCall::SELECTOR => {
            let call = processMalleableAtomicMatchSettleCall::abi_decode(tx_data)?;
            let nullifier = call.internalPartyPayload.validReblindStatement.originalSharesNullifier;
            Ok(u256_to_scalar(nullifier))
        },
        sponsorAtomicMatchSettleCall::SELECTOR => {
            let call = sponsorAtomicMatchSettleCall::abi_decode(tx_data)?;
            let nullifier =
                call.internalPartyMatchPayload.validReblindStatement.originalSharesNullifier;
            Ok(u256_to_scalar(nullifier))
        },
        sponsorMalleableAtomicMatchSettleCall::SELECTOR => {
            let call = sponsorMalleableAtomicMatchSettleCall::abi_decode(tx_data)?;
            let nullifier =
                call.internalPartyMatchPayload.validReblindStatement.originalSharesNullifier;
            Ok(u256_to_scalar(nullifier))
        },
        _ => Err(AuthServerError::serde("Invalid selector for settlement tx")),
    }
}
