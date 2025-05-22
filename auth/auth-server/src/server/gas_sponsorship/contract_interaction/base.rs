//! Contract interaction helpers for Base
//! Logic for interacting with the gas sponsor contract

use alloy::signers::k256::ecdsa::SigningKey;
use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{SolCall, SolValue};
use renegade_api::http::external_match::{ExternalMatchResponse, MalleableExternalMatchResponse};
use renegade_solidity_abi::IDarkpool::{
    processAtomicMatchSettleCall, processMalleableAtomicMatchSettleCall,
    sponsorAtomicMatchSettleCall, sponsorMalleableAtomicMatchSettleCall,
};

use crate::{
    error::AuthServerError,
    server::{
        helpers::{get_selector, sign_message},
        Server,
    },
};

// ---------------
// | ABI Helpers |
// ---------------

/// Convert a `processAtomicMatchSettle` calldata into a
/// `sponsorAtomicMatchSettleWithRefundOptions` call
fn sponsored_atomic_match_calldata(
    calldata: &[u8],
    refund_address: Address,
    nonce: U256,
    refund_native_eth: bool,
    refund_amount: U256,
    signature: Bytes,
) -> Result<sponsorAtomicMatchSettleCall, AuthServerError> {
    let processAtomicMatchSettleCall {
        receiver,
        internalPartyPayload,
        matchSettleStatement,
        proofs,
        linkingProofs,
    } = processAtomicMatchSettleCall::abi_decode(calldata)
        .map_err(AuthServerError::gas_sponsorship)?;

    Ok(sponsorAtomicMatchSettleCall {
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
    })
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
) -> Result<sponsorMalleableAtomicMatchSettleCall, AuthServerError> {
    let processMalleableAtomicMatchSettleCall {
        quoteAmount,
        baseAmount,
        receiver,
        internalPartyPayload,
        matchSettleStatement,
        proofs,
        linkingProofs,
    } = processMalleableAtomicMatchSettleCall::abi_decode(calldata)
        .map_err(AuthServerError::gas_sponsorship)?;

    Ok(sponsorMalleableAtomicMatchSettleCall {
        quoteAmount,
        baseAmount,
        receiver,
        internalPartyMatchPayload: internalPartyPayload,
        malleableMatchSettleStatement: matchSettleStatement,
        matchProofs: proofs,
        matchLinkingProofs: linkingProofs,
        refundAddress: refund_address,
        nonce,
        refundNativeEth: refund_native_eth,
        refundAmount: refund_amount,
        signature,
    })
}

// ---------------
// | Server Impl |
// ---------------

impl Server {
    /// Generate the calldata for sponsoring the given match via the gas sponsor
    pub(crate) fn generate_gas_sponsor_calldata(
        &self,
        external_match_resp: &ExternalMatchResponse,
        refund_address: Address,
        refund_native_eth: bool,
        refund_amount: U256,
    ) -> Result<Bytes, AuthServerError> {
        let (nonce, signature) = gen_signed_sponsorship_nonce(
            refund_address,
            refund_amount,
            &self.gas_sponsor_auth_key,
        )?;

        let tx = &external_match_resp.match_bundle.settlement_tx;
        let calldata = tx.input.input().unwrap_or_default();
        let selector = get_selector(calldata)?;

        let gas_sponsor_call = match selector {
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

        let calldata = gas_sponsor_call.abi_encode().into();
        Ok(calldata)
    }

    /// Generate the calldata for sponsoring the given malleable match bundle
    pub(crate) fn generate_gas_sponsor_malleable_calldata(
        &self,
        external_match_resp: &MalleableExternalMatchResponse,
        refund_address: Address,
        refund_native_eth: bool,
        refund_amount: U256,
    ) -> Result<Bytes, AuthServerError> {
        // Sign a sponsorship permit
        let (nonce, signature) = gen_signed_sponsorship_nonce(
            refund_address,
            refund_amount,
            &self.gas_sponsor_auth_key,
        )?;

        // Parse the calldata and translate it into a gas sponsorship call
        let tx = &external_match_resp.match_bundle.settlement_tx;
        let calldata = tx.input.input().unwrap_or_default();
        let selector = get_selector(calldata)?;

        let gas_sponsor_call = match selector {
            processMalleableAtomicMatchSettleCall::SELECTOR => {
                sponsored_malleable_atomic_match_calldata(
                    calldata,
                    refund_address,
                    nonce,
                    refund_native_eth,
                    refund_amount,
                    signature,
                )
            },
            _ => {
                return Err(AuthServerError::gas_sponsorship("invalid selector"));
            },
        }?;

        let calldata = gas_sponsor_call.abi_encode().into();
        Ok(calldata)
    }
}

// -----------
// | Helpers |
// -----------

/// Generate a random nonce for gas sponsorship, signing it along with
/// the provided refund address and the refund amount
fn gen_signed_sponsorship_nonce(
    refund_address: Address,
    refund_amount: U256,
    gas_sponsor_auth_key: &SigningKey,
) -> Result<(U256, Bytes), AuthServerError> {
    // Generate a random sponsorship nonce then sign the message
    let nonce = U256::random();
    let message = (nonce, refund_address, refund_amount).abi_encode();
    let signature = sign_message(&message, gas_sponsor_auth_key)?.into();

    Ok((nonce, signature))
}
