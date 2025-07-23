//! Arbitrum ABI helpers

use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use renegade_circuit_types::fees::FeeTake;
use renegade_constants::Scalar;
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
            conversion::{
                to_circuit_bounded_match_result, to_circuit_external_match_result,
                to_circuit_fee_rates,
            },
            types::{ValidMalleableMatchSettleAtomicStatement, ValidMatchSettleAtomicStatement},
        },
        helpers::deserialize_calldata,
    },
    conversion::u256_to_amount,
};

use crate::{
    chain_events::{
        abis::{
            compute_malleable_match_internal_fee_take, compute_standard_match_internal_fee_take,
            ExternalMatch,
        },
        error::OnChainEventListenerError,
    },
    server::helpers::get_selector,
};

/// Parse an external match from darkpool calldata,
/// returning it alongside the internal party's fee take
pub(crate) fn parse_external_match(
    calldata: &[u8],
) -> Result<Option<(ExternalMatch, FeeTake)>, OnChainEventListenerError> {
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

/// Parse an external match from a regular atomic match settle call,
/// returning it alongside the internal party's fee take
fn parse_standard_match(
    statement_bytes: &[u8],
) -> Result<(ExternalMatch, FeeTake), OnChainEventListenerError> {
    let statement: ValidMatchSettleAtomicStatement = deserialize_calldata(statement_bytes)?;
    let match_res = to_circuit_external_match_result(&statement.match_result)
        .map_err(OnChainEventListenerError::darkpool)?;

    let protocol_fee_scalar = Scalar::new(statement.protocol_fee);
    let internal_fee_take =
        compute_standard_match_internal_fee_take(protocol_fee_scalar, &match_res);

    Ok((ExternalMatch::Standard(match_res), internal_fee_take))
}

/// Parse an external match from a malleable atomic match settle call
fn parse_malleable_match(
    base_amount: U256,
    statement_bytes: &[u8],
) -> Result<(ExternalMatch, FeeTake), OnChainEventListenerError> {
    let base_amt = u256_to_amount(base_amount).map_err(OnChainEventListenerError::darkpool)?;
    let statement: ValidMalleableMatchSettleAtomicStatement =
        deserialize_calldata(statement_bytes)?;
    let match_res = to_circuit_bounded_match_result(&statement.match_result)
        .map_err(OnChainEventListenerError::darkpool)?;

    let internal_fee_rate = to_circuit_fee_rates(&statement.internal_fee_rates)
        .map_err(OnChainEventListenerError::darkpool)?;

    let internal_fee_take =
        compute_malleable_match_internal_fee_take(internal_fee_rate, &match_res, base_amt)?;

    Ok((ExternalMatch::Malleable(match_res, base_amt), internal_fee_take))
}
