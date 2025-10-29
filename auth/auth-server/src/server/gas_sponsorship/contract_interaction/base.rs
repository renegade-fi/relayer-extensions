//! Contract interaction helpers for Base
//! Logic for interacting with the gas sponsor contract

use alloy::signers::k256::ecdsa::SigningKey;
use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{SolCall, SolValue};
use renegade_api::http::external_match::{ExternalMatchResponse, MalleableExternalMatchResponse};
use renegade_solidity_abi::IDarkpool::{
    executeMalleableAtomicMatchWithInputCall, processAtomicMatchSettleCall,
    processMalleableAtomicMatchSettleCall, sponsorAtomicMatchSettleCall,
    sponsorMalleableAtomicMatchSettleCall,
};

use crate::{
    error::AuthServerError,
    server::{
        Server,
        helpers::{get_selector, sign_message},
    },
};

// ---------------
// | ABI Helpers |
// ---------------

/// The direction of an external match for an internal party on the buy side
///
/// The sol macro encodes these enums as u8 values, so we need to match on
/// integer values.
const DIRECTION_INTERNAL_PARTY_BUY: u8 = 0;
/// The direction of an external match for an internal party on the sell side
const DIRECTION_INTERNAL_PARTY_SELL: u8 = 1;

/// Convert a `processAtomicMatchSettle` calldata into a
/// `sponsorAtomicMatchSettleWithRefundOptions` call
fn sponsored_atomic_match_calldata(
    calldata: &[u8],
    refund_address: Address,
    nonce: U256,
    refund_native_eth: bool,
    refund_amount: U256,
    signature: Bytes,
) -> Result<Bytes, AuthServerError> {
    let processAtomicMatchSettleCall {
        receiver,
        internalPartyPayload,
        matchSettleStatement,
        proofs,
        linkingProofs,
    } = processAtomicMatchSettleCall::abi_decode(calldata)
        .map_err(AuthServerError::gas_sponsorship)?;

    let call = sponsorAtomicMatchSettleCall {
        receiver,
        internalPartyMatchPayload: internalPartyPayload,
        validMatchSettleAtomicStatement: matchSettleStatement,
        matchProofs: proofs,
        matchLinkingProofs: linkingProofs,
        refundAddress: refund_address,
        nonce,
        refundNativeEth: refund_native_eth,
        refundAmount: refund_amount,
        signature,
    };

    Ok(call.abi_encode().into())
}

/// Convert a `processMalleableAtomicMatchSettle` calldata into a
/// `sponsorMalleableAtomicMatchSettleWithRefundOptions` call
fn sponsored_malleable_atomic_match_calldata(
    calldata: &[u8],
    refund_address: Address,
    nonce: U256,
    refund_native_eth: bool,
    refund_amount: U256,
    signature: Bytes,
    use_malleable_match_connector: bool,
) -> Result<Bytes, AuthServerError> {
    // Decode the original call
    let original_call = processMalleableAtomicMatchSettleCall::abi_decode(calldata)
        .map_err(AuthServerError::gas_sponsorship)?;

    // If the user requested their order to be routed through the connector,
    // use calldata for the connector's ABI
    if use_malleable_match_connector {
        return sponsored_malleable_atomic_match_calldata_with_connector(
            original_call,
            refund_address,
            nonce,
            refund_native_eth,
            refund_amount,
            signature,
        );
    }

    // Otherwise, route directly to the gas sponsor
    let call = sponsorMalleableAtomicMatchSettleCall {
        quoteAmount: original_call.quoteAmount,
        baseAmount: original_call.baseAmount,
        receiver: original_call.receiver,
        internalPartyMatchPayload: original_call.internalPartyPayload,
        malleableMatchSettleStatement: original_call.matchSettleStatement,
        matchProofs: original_call.proofs,
        matchLinkingProofs: original_call.linkingProofs,
        refundAddress: refund_address,
        nonce,
        refundNativeEth: refund_native_eth,
        refundAmount: refund_amount,
        signature,
    };
    Ok(call.abi_encode().into())
}

/// Convert a `processMalleableAtomicMatchSettle` calldata into an
/// `executeMalleableAtomicMatchWithInputCall` call
///
/// The connector uses the same ABI as the gas sponsor with the exception that
/// it replaces the base and quote amounts with a single `inputAmount` field.
/// Therefore, this method must determine the input amount based on the original
/// call and use that in place of the base and quote amounts.
fn sponsored_malleable_atomic_match_calldata_with_connector(
    original_call: processMalleableAtomicMatchSettleCall,
    refund_address: Address,
    nonce: U256,
    refund_native_eth: bool,
    refund_amount: U256,
    signature: Bytes,
) -> Result<Bytes, AuthServerError> {
    let direction = original_call.matchSettleStatement.matchResult.direction;
    let input_amount = match direction {
        // Internal party buys the base, so the input is the base amount
        DIRECTION_INTERNAL_PARTY_BUY => original_call.baseAmount,
        // Internal party buys the quote, so the input is the quote amount
        DIRECTION_INTERNAL_PARTY_SELL => original_call.quoteAmount,
        _ => return Err(AuthServerError::gas_sponsorship("invalid match direction")),
    };

    let call = executeMalleableAtomicMatchWithInputCall {
        inputAmount: input_amount,
        receiver: original_call.receiver,
        internalPartyMatchPayload: original_call.internalPartyPayload,
        malleableMatchSettleStatement: original_call.matchSettleStatement,
        matchProofs: original_call.proofs,
        matchLinkingProofs: original_call.linkingProofs,
        refundAddress: refund_address,
        nonce,
        refundNativeEth: refund_native_eth,
        refundAmount: refund_amount,
        signature,
    };

    Ok(call.abi_encode().into())
}

// ---------------
// | Server Impl |
// ---------------

impl Server {
    /// Generate the calldata for sponsoring the given match via the gas
    /// sponsor
    pub(crate) fn generate_gas_sponsor_calldata(
        &self,
        external_match_resp: &ExternalMatchResponse,
        refund_address: Address,
        refund_native_eth: bool,
        refund_amount: U256,
        nonce: U256,
    ) -> Result<Bytes, AuthServerError> {
        let signature = sign_sponsorship_nonce(
            refund_address,
            refund_amount,
            nonce,
            &self.gas_sponsor_auth_key,
        )?;

        let tx = &external_match_resp.match_bundle.settlement_tx;
        let calldata = tx.input.input().unwrap_or_default();
        let selector = get_selector(calldata)?;

        let calldata = match selector {
            processAtomicMatchSettleCall::SELECTOR => sponsored_atomic_match_calldata(
                calldata,
                refund_address,
                nonce,
                refund_native_eth,
                refund_amount,
                signature,
            ),
            _ => {
                return Err(AuthServerError::gas_sponsorship("invalid selector"));
            },
        }?;
        Ok(calldata)
    }

    /// Generate the calldata for sponsoring the given malleable match bundle
    pub(crate) fn generate_gas_sponsor_malleable_calldata(
        &self,
        external_match_resp: &MalleableExternalMatchResponse,
        refund_address: Address,
        refund_native_eth: bool,
        refund_amount: U256,
        nonce: U256,
        use_malleable_match_connector: bool,
    ) -> Result<Bytes, AuthServerError> {
        // Sign a sponsorship permit
        let signature = sign_sponsorship_nonce(
            refund_address,
            refund_amount,
            nonce,
            &self.gas_sponsor_auth_key,
        )?;

        // Parse the calldata and translate it into a gas sponsorship call
        let tx = &external_match_resp.match_bundle.settlement_tx;
        let calldata = tx.input.input().unwrap_or_default();
        let selector = get_selector(calldata)?;

        let calldata = match selector {
            processMalleableAtomicMatchSettleCall::SELECTOR => {
                sponsored_malleable_atomic_match_calldata(
                    calldata,
                    refund_address,
                    nonce,
                    refund_native_eth,
                    refund_amount,
                    signature,
                    use_malleable_match_connector,
                )
            },
            _ => {
                return Err(AuthServerError::gas_sponsorship("invalid selector"));
            },
        }?;
        Ok(calldata)
    }
}

// -----------
// | Helpers |
// -----------

/// Sign the provided sponsorship nonce, along with
/// the refund address and the refund amount
fn sign_sponsorship_nonce(
    refund_address: Address,
    refund_amount: U256,
    nonce: U256,
    gas_sponsor_auth_key: &SigningKey,
) -> Result<Bytes, AuthServerError> {
    let message = (nonce, refund_address, refund_amount).abi_encode();
    let signature = sign_message(&message, gas_sponsor_auth_key)?.into();

    Ok(signature)
}
