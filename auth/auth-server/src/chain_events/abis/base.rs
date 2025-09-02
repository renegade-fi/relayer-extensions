//! Base ABI helpers

use alloy_sol_types::{SolCall, sol};
use renegade_darkpool_client::{base::conversion::ToCircuitType, conversion::u256_to_amount};
use renegade_solidity_abi::IDarkpool::{
    processAtomicMatchSettleCall, processMalleableAtomicMatchSettleCall,
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
        event SponsoredExternalMatch(uint256 amount, address token, uint256 indexed nonce);
        event NonceUsed(uint256 indexed nonce);
    }
}

/// Parse an external match from darkpool calldata
pub(crate) fn parse_external_match(
    calldata: &[u8],
) -> Result<Option<ExternalMatch>, OnChainEventListenerError> {
    let selector = get_selector(calldata)?;
    let match_res = match selector {
        processAtomicMatchSettleCall::SELECTOR => {
            let call = processAtomicMatchSettleCall::abi_decode(calldata)?;
            let match_res = call.matchSettleStatement.matchResult.to_circuit_type()?;
            ExternalMatch::Standard(match_res)
        },
        processMalleableAtomicMatchSettleCall::SELECTOR => {
            let call = processMalleableAtomicMatchSettleCall::abi_decode(calldata)?;
            let match_res = call.matchSettleStatement.matchResult.to_circuit_type()?;
            let base_amt =
                u256_to_amount(call.baseAmount).map_err(OnChainEventListenerError::darkpool)?;

            ExternalMatch::Malleable(match_res, base_amt)
        },
        _ => return Ok(None),
    };

    Ok(Some(match_res))
}
