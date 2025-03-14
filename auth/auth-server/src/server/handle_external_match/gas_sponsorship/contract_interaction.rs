//! Logic for interacting with the gas sponsor contract

use alloy_primitives::{Address as AlloyAddress, Bytes as AlloyBytes, U256 as AlloyU256};
use alloy_sol_types::{sol, SolCall};
use bytes::Bytes;
use ethers::contract::abigen;
use renegade_api::http::external_match::ExternalMatchResponse;
use renegade_arbitrum_client::abi::{
    processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall,
};

use crate::{
    error::AuthServerError,
    server::{
        helpers::{gen_signed_sponsorship_nonce, get_selector},
        Server,
    },
};

// -------
// | ABI |
// -------

// The ABI for gas sponsorship events
abigen!(
    GasSponsorContract,
    r#"[
        event SponsoredExternalMatch(uint256 indexed amount, address indexed token, uint256 indexed nonce)
    ]"#
);

// The ABI for gas sponsorship functions
sol! {
    function sponsorAtomicMatchSettleWithRefundOptions(address receiver, bytes internal_party_match_payload, bytes valid_match_settle_atomic_statement, bytes match_proofs, bytes match_linking_proofs, address refund_address, uint256 nonce, bool refund_native_eth, uint256 refund_amount, bytes signature) external payable;
}

impl sponsorAtomicMatchSettleWithRefundOptionsCall {
    /// Create a `sponsorAtomicMatchSettleWithRefundOptions` call from
    /// `processAtomicMatchSettle` calldata
    pub fn from_process_atomic_match_settle_calldata(
        calldata: &[u8],
        refund_address: AlloyAddress,
        nonce: AlloyU256,
        refund_native_eth: bool,
        refund_amount: AlloyU256,
        signature: AlloyBytes,
    ) -> Result<Self, AuthServerError> {
        let processAtomicMatchSettleCall {
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
        } = processAtomicMatchSettleCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(sponsorAtomicMatchSettleWithRefundOptionsCall {
            receiver: AlloyAddress::ZERO,
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
            refund_address,
            nonce,
            refund_native_eth,
            refund_amount,
            signature,
        })
    }

    /// Create a `sponsorAtomicMatchSettleWithRefundOptions` call from
    /// `processAtomicMatchSettleWithReceiver` calldata
    pub fn from_process_atomic_match_settle_with_receiver_calldata(
        calldata: &[u8],
        refund_address: AlloyAddress,
        nonce: AlloyU256,
        refund_native_eth: bool,
        refund_amount: AlloyU256,
        signature: AlloyBytes,
    ) -> Result<Self, AuthServerError> {
        let processAtomicMatchSettleWithReceiverCall {
            receiver,
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
        } = processAtomicMatchSettleWithReceiverCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(sponsorAtomicMatchSettleWithRefundOptionsCall {
            receiver,
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
            refund_address,
            nonce,
            refund_native_eth,
            refund_amount,
            signature,
        })
    }
}

// ---------------
// | Server Impl |
// ---------------

impl Server {
    /// Generate the calldata for sponsoring the given match via the gas sponsor
    pub(crate) fn generate_gas_sponsor_calldata(
        &self,
        external_match_resp: &ExternalMatchResponse,
        refund_address: AlloyAddress,
        refund_native_eth: bool,
        refund_amount: AlloyU256,
    ) -> Result<Bytes, AuthServerError> {
        let (nonce, signature) = gen_signed_sponsorship_nonce(
            refund_address,
            refund_amount,
            &self.gas_sponsor_auth_key,
        )?;

        let calldata = external_match_resp
            .match_bundle
            .settlement_tx
            .data()
            .ok_or(AuthServerError::gas_sponsorship("expected calldata"))?;

        let selector = get_selector(calldata)?;

        let gas_sponsor_call = match selector {
            processAtomicMatchSettleCall::SELECTOR => {
                sponsorAtomicMatchSettleWithRefundOptionsCall::from_process_atomic_match_settle_calldata(
                    calldata,
                    refund_address,
                    nonce,
                    refund_native_eth,
                    refund_amount,
                    signature,
                )
            },
            processAtomicMatchSettleWithReceiverCall::SELECTOR => {
                sponsorAtomicMatchSettleWithRefundOptionsCall::from_process_atomic_match_settle_with_receiver_calldata(
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
