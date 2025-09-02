//! Arbitrum ABI helpers

use alloy_primitives::U256;
use alloy_sol_types::{SolCall, sol};
use renegade_darkpool_client::{
    arbitrum::{
        abi::{
            Darkpool::{
                processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall,
                processMalleableAtomicMatchSettleCall,
                processMalleableAtomicMatchSettleWithReceiverCall,
            },
            PROCESS_ATOMIC_MATCH_SETTLE_SELECTOR,
            PROCESS_ATOMIC_MATCH_SETTLE_WITH_RECEIVER_SELECTOR,
            PROCESS_MALLEABLE_ATOMIC_MATCH_SETTLE_SELECTOR,
            PROCESS_MALLEABLE_ATOMIC_MATCH_SETTLE_WITH_RECEIVER_SELECTOR,
        },
        contract_types::{
            conversion::{to_circuit_bounded_match_result, to_circuit_external_match_result},
            types::{ValidMalleableMatchSettleAtomicStatement, ValidMatchSettleAtomicStatement},
        },
        helpers::deserialize_calldata,
    },
    conversion::u256_to_amount,
};

use crate::{
    chain_events::{abis::ExternalMatch, error::OnChainEventListenerError},
    server::helpers::get_selector,
};

// -------
// | ABI |
// -------

// The ABI for gas sponsorship events
sol! {
    contract GasSponsorContract {
        event SponsoredExternalMatch(uint256 indexed amount, address indexed token, uint256 indexed nonce);
        event NonceUsed(uint256 indexed nonce);
    }
}

/// Parse an external match from darkpool calldata
pub(crate) fn parse_external_match(
    calldata: &[u8],
) -> Result<Option<ExternalMatch>, OnChainEventListenerError> {
    let selector = get_selector(calldata)?;
    // Parse the `VALID MATCH SETTLE ATOMIC` statement from the calldata
    let match_res = match selector {
        PROCESS_ATOMIC_MATCH_SETTLE_SELECTOR => {
            let call = processAtomicMatchSettleCall::abi_decode(calldata)?;
            parse_standard_match(&call.valid_match_settle_atomic_statement)
        },
        PROCESS_ATOMIC_MATCH_SETTLE_WITH_RECEIVER_SELECTOR => {
            let call = processAtomicMatchSettleWithReceiverCall::abi_decode(calldata)?;
            parse_standard_match(&call.valid_match_settle_atomic_statement)
        },
        PROCESS_MALLEABLE_ATOMIC_MATCH_SETTLE_WITH_RECEIVER_SELECTOR => {
            let call = processMalleableAtomicMatchSettleWithReceiverCall::abi_decode(calldata)?;
            parse_malleable_match(call.base_amount, &call.valid_match_settle_statement)
        },
        PROCESS_MALLEABLE_ATOMIC_MATCH_SETTLE_SELECTOR => {
            let call = processMalleableAtomicMatchSettleCall::abi_decode(calldata)?;
            parse_malleable_match(call.base_amount, &call.valid_match_settle_statement)
        },
        _ => return Ok(None),
    }?;

    Ok(Some(match_res))
}

/// Parse an external match from a regular atomic match settle call
fn parse_standard_match(
    statement_bytes: &[u8],
) -> Result<ExternalMatch, OnChainEventListenerError> {
    let statement: ValidMatchSettleAtomicStatement = deserialize_calldata(statement_bytes)?;
    let match_res = to_circuit_external_match_result(&statement.match_result)
        .map_err(OnChainEventListenerError::darkpool)?;

    Ok(ExternalMatch::Standard(match_res))
}

/// Parse an external match from a malleable atomic match settle call
fn parse_malleable_match(
    base_amount: U256,
    statement_bytes: &[u8],
) -> Result<ExternalMatch, OnChainEventListenerError> {
    let base_amt = u256_to_amount(base_amount).map_err(OnChainEventListenerError::darkpool)?;
    let statement: ValidMalleableMatchSettleAtomicStatement =
        deserialize_calldata(statement_bytes)?;
    let match_res = to_circuit_bounded_match_result(&statement.match_result)
        .map_err(OnChainEventListenerError::darkpool)?;

    Ok(ExternalMatch::Malleable(match_res, base_amt))
}
