//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::Address;
use alloy_sol_types::{sol, SolCall};
use auth_server_api::ExternalMatchResponse;
use bytes::Bytes;
use ethers::contract::abigen;
use ethers::types::{transaction::eip2718::TypedTransaction, TxHash, U256};
use ethers::utils::format_ether;
use http::header::CONTENT_LENGTH;
use http::Response;
use renegade_arbitrum_client::abi::{
    processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall,
};
use tracing::{info, warn};

use renegade_api::http::external_match::ExternalMatchResponse as RelayerExternalMatchResponse;

use super::Server;
use crate::error::AuthServerError;
use crate::server::helpers::{gen_signed_sponsorship_nonce, get_selector};
use crate::telemetry::helpers::record_gas_sponsorship_metrics;

// -------
// | ABI |
// -------

// The ABI for gas sponsorship functions
sol! {
    function sponsorAtomicMatchSettle(bytes memory internal_party_match_payload, bytes memory valid_match_settle_atomic_statement, bytes memory match_proofs, bytes memory match_linking_proofs, address memory refund_address, uint256 memory nonce, bytes memory signature) external payable;
    function sponsorAtomicMatchSettleWithReceiver(address receiver, bytes memory internal_party_match_payload, bytes memory valid_match_settle_atomic_statement, bytes memory match_proofs, bytes memory match_linking_proofs, address memory refund_address, uint256 memory nonce, bytes memory signature) external payable;
}

// The ABI for gas sponsorship events
abigen!(
    GasSponsorContract,
    r#"[
        event SponsoredExternalMatch(uint256 indexed amount, uint256 indexed nonce)
    ]"#
);

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Mutate a quote assembly response to invoke gas sponsorship
    pub(crate) fn mutate_response_for_gas_sponsorship(
        &self,
        resp: &mut Response<Bytes>,
        is_sponsored: bool,
        refund_address: Address,
    ) -> Result<(), AuthServerError> {
        let mut relayer_external_match_resp: RelayerExternalMatchResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;

        relayer_external_match_resp.match_bundle.settlement_tx.set_to(self.gas_sponsor_address);

        if is_sponsored {
            info!("Sponsoring match bundle via gas sponsor");

            let gas_sponsor_calldata = self
                .generate_gas_sponsor_calldata(&relayer_external_match_resp, refund_address)?
                .into();

            relayer_external_match_resp.match_bundle.settlement_tx.set_data(gas_sponsor_calldata);
        }

        let external_match_resp = ExternalMatchResponse {
            match_bundle: relayer_external_match_resp.match_bundle,
            is_sponsored,
        };

        let body =
            Bytes::from(serde_json::to_vec(&external_match_resp).map_err(AuthServerError::serde)?);

        resp.headers_mut().insert(CONTENT_LENGTH, body.len().into());
        *resp.body_mut() = body;

        Ok(())
    }

    /// Generate the calldata for sponsoring the given match via the gas sponsor
    fn generate_gas_sponsor_calldata(
        &self,
        external_match_resp: &RelayerExternalMatchResponse,
        refund_address: Address,
    ) -> Result<Bytes, AuthServerError> {
        let calldata = external_match_resp
            .match_bundle
            .settlement_tx
            .data()
            .ok_or(AuthServerError::gas_sponsorship("expected calldata"))?;

        let selector = get_selector(calldata)?;

        let gas_sponsor_calldata = match selector {
            processAtomicMatchSettleCall::SELECTOR => {
                self.sponsor_atomic_match_settle_call(calldata, refund_address)
            },
            processAtomicMatchSettleWithReceiverCall::SELECTOR => {
                self.sponsor_atomic_match_settle_with_receiver_call(calldata, refund_address)
            },
            _ => {
                return Err(AuthServerError::gas_sponsorship("invalid selector"));
            },
        }?;

        Ok(gas_sponsor_calldata)
    }

    /// Create a `sponsorAtomicMatchSettle` call from `processAtomicMatchSettle`
    /// calldata
    fn sponsor_atomic_match_settle_call(
        &self,
        calldata: &[u8],
        refund_address: Address,
    ) -> Result<Bytes, AuthServerError> {
        let call = processAtomicMatchSettleCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        let (nonce, signature) =
            gen_signed_sponsorship_nonce(refund_address, &self.gas_sponsor_auth_key)?;

        let sponsored_call = sponsorAtomicMatchSettleCall {
            internal_party_match_payload: call.internal_party_match_payload,
            valid_match_settle_atomic_statement: call.valid_match_settle_atomic_statement,
            match_proofs: call.match_proofs,
            match_linking_proofs: call.match_linking_proofs,
            refund_address,
            nonce,
            signature,
        };

        Ok(sponsored_call.abi_encode().into())
    }

    /// Create a `sponsorAtomicMatchSettleWithReceiver` call from
    /// `processAtomicMatchSettleWithReceiver` calldata
    fn sponsor_atomic_match_settle_with_receiver_call(
        &self,
        calldata: &[u8],
        refund_address: Address,
    ) -> Result<Bytes, AuthServerError> {
        let call = processAtomicMatchSettleWithReceiverCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        let (nonce, signature) =
            gen_signed_sponsorship_nonce(refund_address, &self.gas_sponsor_auth_key)?;

        let sponsored_call = sponsorAtomicMatchSettleWithReceiverCall {
            receiver: call.receiver,
            internal_party_match_payload: call.internal_party_match_payload,
            valid_match_settle_atomic_statement: call.valid_match_settle_atomic_statement,
            match_proofs: call.match_proofs,
            match_linking_proofs: call.match_linking_proofs,
            refund_address,
            nonce,
            signature,
        };

        Ok(sponsored_call.abi_encode().into())
    }

    /// Get the amount of Ether spent to sponsor the given settlement
    /// transaction, and the associated transaction hash
    async fn get_sponsorship_amount_and_tx(
        &self,
        settlement_tx: &TypedTransaction,
    ) -> Result<Option<(U256, TxHash)>, AuthServerError> {
        // Parse the nonce from the TX calldata
        let calldata =
            settlement_tx.data().ok_or(AuthServerError::gas_sponsorship("expected calldata"))?;

        let selector = get_selector(calldata)?;

        let nonce = match selector {
            sponsorAtomicMatchSettleCall::SELECTOR => {
                Self::get_nonce_from_sponsor_atomic_match_calldata(calldata)?
            },
            sponsorAtomicMatchSettleWithReceiverCall::SELECTOR => {
                Self::get_nonce_from_sponsor_atomic_match_with_receiver_calldata(calldata)?
            },
            _ => {
                return Err(AuthServerError::gas_sponsorship("invalid selector"));
            },
        };

        // Search for the `AmountSponsored` event for the given nonce
        let filter =
            GasSponsorContract::new(self.gas_sponsor_address, self.arbitrum_client.client())
                .event::<SponsoredExternalMatchFilter>()
                .address(self.gas_sponsor_address.into())
                .topic2(nonce)
                .from_block(self.start_block_num);

        let events = filter.query_with_meta().await.map_err(AuthServerError::gas_sponsorship)?;

        // If no event was found, we assume that gas was not sponsored for this nonce.
        // This could be the case if the gas sponsor was underfunded or paused.
        let amount_sponsored_with_tx =
            events.last().map(|(event, meta)| (event.amount, meta.transaction_hash));

        if amount_sponsored_with_tx.is_none() {
            warn!("No gas sponsorship event found for nonce: {}", nonce);
        }

        Ok(amount_sponsored_with_tx)
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    pub async fn record_settled_match_sponsorship(
        &self,
        match_resp: &ExternalMatchResponse,
        key: String,
        request_id: String,
    ) -> Result<(), AuthServerError> {
        if match_resp.is_sponsored
            && let Some((gas_cost, tx_hash)) =
                self.get_sponsorship_amount_and_tx(&match_resp.match_bundle.settlement_tx).await?
        {
            // Convert wei to ether using format_ether, then parse to f64
            let gas_cost_eth: f64 =
                format_ether(gas_cost).parse().map_err(AuthServerError::custom)?;

            let eth_price: f64 = self
                .price_reporter_client
                .get_eth_price()
                .await
                .map_err(AuthServerError::custom)?;

            let gas_sponsorship_value = eth_price * gas_cost_eth;

            self.rate_limiter.record_gas_sponsorship(key, gas_sponsorship_value).await;

            record_gas_sponsorship_metrics(gas_sponsorship_value, tx_hash, request_id);
        }

        Ok(())
    }

    /// Get the nonce from `sponsorAtomicMatchSettle` calldata
    fn get_nonce_from_sponsor_atomic_match_calldata(
        calldata: &[u8],
    ) -> Result<U256, AuthServerError> {
        let call = sponsorAtomicMatchSettleCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(U256::from_big_endian(&call.nonce.to_be_bytes_vec()))
    }

    /// Get the nonce from `sponsorAtomicMatchSettleWithReceiver` calldata
    fn get_nonce_from_sponsor_atomic_match_with_receiver_calldata(
        calldata: &[u8],
    ) -> Result<U256, AuthServerError> {
        let call = sponsorAtomicMatchSettleWithReceiverCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(U256::from_big_endian(&call.nonce.to_be_bytes_vec()))
    }
}
