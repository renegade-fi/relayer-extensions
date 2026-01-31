//! Helpers for interacting with the gas sponsorship contract

use alloy_primitives::{Bytes, U256};
use alloy_sol_types::{SolCall, SolValue};
use auth_server_api::GasSponsorshipInfo;
use renegade_external_api::http::external_match::ExternalMatchResponse;
use renegade_solidity_abi::v2::IDarkpoolV2;

use crate::{error::AuthServerError, server::Server};

use super::super::helpers::sign_message;

impl Server {
    /// Generate calldata for a gas sponsorship transaction
    pub(crate) fn generate_gas_sponsor_calldata(
        &self,
        external_match_resp: &ExternalMatchResponse,
        info: &GasSponsorshipInfo,
        nonce: U256,
    ) -> Result<Bytes, AuthServerError> {
        let original_tx = &external_match_resp.match_bundle.settlement_tx;
        let tx_data = original_tx.input.input().unwrap_or_default();
        let tx = IDarkpoolV2::settleExternalMatchCall::abi_decode(tx_data)?;

        let options = self.get_gas_sponsor_options(nonce, info)?;
        let sponsor_call = IDarkpoolV2::sponsorExternalMatchCall {
            externalPartyAmountIn: tx.externalPartyAmountIn,
            recipient: tx.recipient,
            matchResult: tx.matchResult,
            internalPartySettlementBundle: tx.internalPartySettlementBundle,
            options,
        };

        Ok(sponsor_call.abi_encode().into())
    }

    /// Get the gas sponsor options for a given bundle
    fn get_gas_sponsor_options(
        &self,
        nonce: U256,
        info: &GasSponsorshipInfo,
    ) -> Result<IDarkpoolV2::GasSponsorOptions, AuthServerError> {
        let signature = self.sign_gas_sponsor_options(nonce, info)?;
        Ok(IDarkpoolV2::GasSponsorOptions {
            refundAddress: info.get_refund_address(),
            refundAmount: info.get_refund_amount(),
            refundNativeEth: info.refund_native_eth,
            nonce,
            signature,
        })
    }

    /// Sign the gas sponsorship options
    ///
    /// The signature is over the ABI-encoded tuple:
    /// `(refundAddress, refundNativeEth, refundAmount, nonce, chainId)`
    ///
    /// This matches the contract's `_assertSponsorshipSignature` verification:
    /// ```solidity
    /// bytes32 messageHash = EfficientHashLib.hash(
    ///     abi.encode(
    ///         options.refundAddress, options.refundNativeEth,
    ///         options.refundAmount, options.nonce, block.chainid
    ///     )
    /// );
    /// ```
    fn sign_gas_sponsor_options(
        &self,
        nonce: U256,
        info: &GasSponsorshipInfo,
    ) -> Result<Bytes, AuthServerError> {
        let refund_address = info.get_refund_address();
        let refund_native_eth = info.refund_native_eth;
        let refund_amount = info.get_refund_amount();
        let chain_id = U256::from(self.chain.chain_id());

        // ABI encode the message as per the Solidity contract
        let message =
            (refund_address, refund_native_eth, refund_amount, nonce, chain_id).abi_encode();

        // Sign the message
        let signature = sign_message(&message, &self.gas_sponsor_auth_key)?;
        Ok(signature.to_vec().into())
    }
}
