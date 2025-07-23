//! Base ABI helpers

use alloy_sol_types::SolCall;
use renegade_circuit_types::fees::FeeTake;
use renegade_darkpool_client::{
    base::conversion::ToCircuitType,
    conversion::{u256_to_amount, u256_to_scalar},
};
use renegade_solidity_abi::IDarkpool::{
    processAtomicMatchSettleCall, processMalleableAtomicMatchSettleCall,
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
    let (match_res, internal_fee_take) = match selector {
        processAtomicMatchSettleCall::SELECTOR => {
            let call = processAtomicMatchSettleCall::abi_decode(calldata)?;
            let match_res = call.matchSettleStatement.matchResult.to_circuit_type()?;

            let protocol_fee_scalar = u256_to_scalar(call.matchSettleStatement.protocolFeeRate);
            let internal_fee_take =
                compute_standard_match_internal_fee_take(protocol_fee_scalar, &match_res);

            (ExternalMatch::Standard(match_res), internal_fee_take)
        },
        processMalleableAtomicMatchSettleCall::SELECTOR => {
            let call = processMalleableAtomicMatchSettleCall::abi_decode(calldata)?;
            let match_res = call.matchSettleStatement.matchResult.to_circuit_type()?;
            let base_amt =
                u256_to_amount(call.baseAmount).map_err(OnChainEventListenerError::darkpool)?;

            let internal_fee_take = compute_malleable_match_internal_fee_take(
                call.matchSettleStatement.internalFeeRates.to_circuit_type()?,
                &match_res,
                base_amt,
            )?;

            (ExternalMatch::Malleable(match_res, base_amt), internal_fee_take)
        },
        _ => return Ok(None),
    };

    Ok(Some((match_res, internal_fee_take)))
}
