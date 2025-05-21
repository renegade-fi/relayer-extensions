//! Logic for interacting with the gas sponsor contract

use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{sol, SolCall};
use renegade_api::http::external_match::{ExternalMatchResponse, MalleableExternalMatchResponse};
use renegade_darkpool_client::arbitrum::abi::Darkpool::{
    processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall,
    processMalleableAtomicMatchSettleCall, processMalleableAtomicMatchSettleWithReceiverCall,
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
sol! {
    contract GasSponsorContract {
        event SponsoredExternalMatch(uint256 indexed amount, address indexed token, uint256 indexed nonce);
    }
}

// The ABI for gas sponsorship functions
sol! {
    function sponsorAtomicMatchSettleWithRefundOptions(
        address receiver,
        bytes internal_party_match_payload,
        bytes valid_match_settle_atomic_statement,
        bytes match_proofs,
        bytes match_linking_proofs,
        address refund_address,
        uint256 nonce,
        bool refund_native_eth,
        uint256 refund_amount,
        bytes signature
    ) external payable returns (uint256);
    function sponsorMalleableAtomicMatchSettleWithRefundOptions(
        uint256 quote_amount,
        uint256 base_amount,
        address receiver,
        bytes memory internal_party_payload,
        bytes memory malleable_match_settle_atomic_statement,
        bytes memory proofs,
        bytes memory linking_proofs,
        address memory refund_address,
        uint256 memory nonce,
        bool memory refund_native_eth,
        uint256 memory refund_amount,
        bytes memory signature
    ) external payable returns (uint256);
}

impl sponsorAtomicMatchSettleWithRefundOptionsCall {
    /// Create a `sponsorAtomicMatchSettleWithRefundOptions` call from
    /// `processAtomicMatchSettle` calldata
    pub fn from_process_atomic_match_settle_calldata(
        calldata: &[u8],
        refund_address: Address,
        nonce: U256,
        refund_native_eth: bool,
        refund_amount: U256,
        signature: Bytes,
    ) -> Result<Self, AuthServerError> {
        let processAtomicMatchSettleCall {
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
        } = processAtomicMatchSettleCall::abi_decode(calldata)
            .map_err(AuthServerError::gas_sponsorship)?;

        Ok(sponsorAtomicMatchSettleWithRefundOptionsCall {
            receiver: Address::ZERO,
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
        refund_address: Address,
        nonce: U256,
        refund_native_eth: bool,
        refund_amount: U256,
        signature: Bytes,
    ) -> Result<Self, AuthServerError> {
        let processAtomicMatchSettleWithReceiverCall {
            receiver,
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
        } = processAtomicMatchSettleWithReceiverCall::abi_decode(calldata)
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

impl sponsorMalleableAtomicMatchSettleWithRefundOptionsCall {
    /// Create a `sponsorMalleableAtomicMatchSettleWithRefundOptions` call from
    /// `processMalleableAtomicMatchSettle` calldata
    pub fn from_process_malleable_atomic_match_settle_calldata(
        calldata: &[u8],
        refund_address: Address,
        nonce: U256,
        refund_native_eth: bool,
        refund_amount: U256,
        signature: Bytes,
    ) -> Result<Self, AuthServerError> {
        let processMalleableAtomicMatchSettleCall {
            quote_amount,
            base_amount,
            internal_party_match_payload,
            valid_match_settle_statement,
            match_proofs,
            match_linking_proofs,
        } = processMalleableAtomicMatchSettleCall::abi_decode(calldata)
            .map_err(AuthServerError::gas_sponsorship)?;

        Ok(sponsorMalleableAtomicMatchSettleWithRefundOptionsCall {
            quote_amount,
            base_amount,
            receiver: Address::ZERO,
            internal_party_payload: internal_party_match_payload,
            malleable_match_settle_atomic_statement: valid_match_settle_statement,
            proofs: match_proofs,
            linking_proofs: match_linking_proofs,
            refund_address,
            nonce,
            refund_native_eth,
            refund_amount,
            signature,
        })
    }

    /// Create a `sponsorMalleableAtomicMatchSettleWithRefundOptions` call from
    /// `processMalleableAtomicMatchSettleWithReceiver` calldata
    pub fn from_process_malleable_atomic_match_settle_with_receiver_calldata(
        calldata: &[u8],
        refund_address: Address,
        nonce: U256,
        refund_native_eth: bool,
        refund_amount: U256,
        signature: Bytes,
    ) -> Result<Self, AuthServerError> {
        let processMalleableAtomicMatchSettleWithReceiverCall {
            quote_amount,
            base_amount,
            receiver,
            internal_party_match_payload,
            valid_match_settle_statement,
            match_proofs,
            match_linking_proofs,
        } = processMalleableAtomicMatchSettleWithReceiverCall::abi_decode(calldata)
            .map_err(AuthServerError::gas_sponsorship)?;

        Ok(sponsorMalleableAtomicMatchSettleWithRefundOptionsCall {
            quote_amount,
            base_amount,
            receiver,
            internal_party_payload: internal_party_match_payload,
            malleable_match_settle_atomic_statement: valid_match_settle_statement,
            proofs: match_proofs,
            linking_proofs: match_linking_proofs,
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
                sponsorMalleableAtomicMatchSettleWithRefundOptionsCall::from_process_malleable_atomic_match_settle_calldata(
                    calldata,
                    refund_address,
                    nonce,
                    refund_native_eth,
                    refund_amount,
                    signature,
                )
            },
            processMalleableAtomicMatchSettleWithReceiverCall::SELECTOR => {
                sponsorMalleableAtomicMatchSettleWithRefundOptionsCall::from_process_malleable_atomic_match_settle_with_receiver_calldata(
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
