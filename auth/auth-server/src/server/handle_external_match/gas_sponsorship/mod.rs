//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::{Address as AlloyAddress, Bytes as AlloyBytes, U256 as AlloyU256};
use alloy_sol_types::{sol, SolCall};
use auth_server_api::SponsoredMatchResponse;
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use bytes::Bytes;
use ethers::{
    contract::abigen,
    types::{transaction::eip2718::TypedTransaction, Address, TxHash, U256},
    utils::WEI_IN_ETHER,
};
use http::{header::CONTENT_LENGTH, Response};
use renegade_arbitrum_client::abi::{
    processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall,
};
use renegade_constants::NATIVE_ASSET_ADDRESS;
use tracing::{info, warn};

use renegade_api::http::external_match::{AtomicMatchApiBundle, ExternalMatchResponse};

use super::Server;
use crate::server::helpers::{
    ethers_u256_to_alloy_u256, gen_signed_sponsorship_nonce, get_nominal_buy_token_price,
    get_selector,
};
use crate::telemetry::helpers::record_gas_sponsorship_metrics;
use crate::{error::AuthServerError, server::helpers::ethers_u256_to_bigdecimal};

// -------
// | ABI |
// -------

// The ABI for gas sponsorship functions
sol! {
    function sponsorAtomicMatchSettleWithRefundOptions(address receiver, bytes internal_party_match_payload, bytes valid_match_settle_atomic_statement, bytes match_proofs, bytes match_linking_proofs, address refund_address, uint256 nonce, bool refund_native_eth, uint256 sponsorship_amount, bytes signature) external payable;
}

// The ABI for gas sponsorship events
abigen!(
    GasSponsorContract,
    r#"[
        event SponsoredExternalMatch(uint256 indexed amount, address indexed token, uint256 indexed nonce)
    ]"#
);

impl sponsorAtomicMatchSettleWithRefundOptionsCall {
    /// Create a `sponsorAtomicMatchSettleWithRefundOptions` call from
    /// `processAtomicMatchSettle` calldata
    pub fn from_process_atomic_match_settle_calldata(
        calldata: &[u8],
        refund_address: AlloyAddress,
        nonce: AlloyU256,
        refund_native_eth: bool,
        sponsorship_amount: AlloyU256,
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
            sponsorship_amount,
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
        sponsorship_amount: AlloyU256,
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
            sponsorship_amount,
            signature,
        })
    }
}

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Mutate a quote assembly response to invoke gas sponsorship
    pub(crate) async fn mutate_response_for_gas_sponsorship(
        &self,
        resp: &mut Response<Bytes>,
        is_sponsored: bool,
        refund_address: AlloyAddress,
        refund_native_eth: bool,
    ) -> Result<(), AuthServerError> {
        let mut relayer_external_match_resp: ExternalMatchResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;

        relayer_external_match_resp.match_bundle.settlement_tx.set_to(self.gas_sponsor_address);

        if is_sponsored {
            info!("Sponsoring match bundle via gas sponsor");

            let conversion_rate = self
                .maybe_fetch_conversion_rate(&relayer_external_match_resp, refund_native_eth)
                .await?;

            let estimated_gas_cost = ethers_u256_to_alloy_u256(self.get_gas_cost_estimate().await);

            let sponsorship_amount = if let Some(conversion_rate) = conversion_rate {
                let wei_in_eth = ethers_u256_to_alloy_u256(WEI_IN_ETHER);
                (estimated_gas_cost / wei_in_eth) * conversion_rate
            } else {
                estimated_gas_cost
            };

            let gas_sponsor_calldata = self
                .generate_gas_sponsor_calldata(
                    &relayer_external_match_resp,
                    refund_address,
                    refund_native_eth,
                    sponsorship_amount,
                )?
                .into();

            relayer_external_match_resp.match_bundle.settlement_tx.set_data(gas_sponsor_calldata);
        }

        let external_match_resp = SponsoredMatchResponse {
            match_bundle: relayer_external_match_resp.match_bundle,
            is_sponsored,
        };

        let body =
            Bytes::from(serde_json::to_vec(&external_match_resp).map_err(AuthServerError::serde)?);

        resp.headers_mut().insert(CONTENT_LENGTH, body.len().into());
        *resp.body_mut() = body;

        Ok(())
    }

    /// Fetch the conversion rate from ETH to the buy-side token in the trade
    /// from the price reporter, if necessary.
    /// The conversion rate is in terms of nominal units of TOKEN per whole ETH.
    #[allow(clippy::unused_async)]
    async fn maybe_fetch_conversion_rate(
        &self,
        external_match_resp: &ExternalMatchResponse,
        refund_native_eth: bool,
    ) -> Result<Option<AlloyU256>, AuthServerError> {
        let buy_mint = &external_match_resp.match_bundle.receive.mint;
        let native_eth_buy = buy_mint.to_lowercase() == NATIVE_ASSET_ADDRESS.to_lowercase();

        // If we're deliberately refunding via native ETH, or the buy-side token
        // is native ETH, we don't need to get a conversion rate
        if refund_native_eth || native_eth_buy {
            return Ok(None);
        }

        // Get ETH price
        let eth_price_f64 = self.price_reporter_client.get_eth_price().await?;
        let eth_price = BigDecimal::from_f64(eth_price_f64)
            .ok_or(AuthServerError::gas_sponsorship("failed to convert ETH price to BigDecimal"))?;

        let buy_token_price = get_nominal_buy_token_price(&external_match_resp.match_bundle)?;

        // Compute conversion rate of nominal units of TOKEN per whole ETH
        let conversion_rate = eth_price / buy_token_price;

        // Convert the scaled rate to a U256. We can use the `BigInt` component of the
        // `BigDecimal` directly because we round to 0 digits after the decimal.
        let (conversion_rate_bigint, _) =
            conversion_rate.round(0 /* round_digits */).into_bigint_and_scale();

        let conversion_rate_u256 = AlloyU256::try_from(conversion_rate_bigint)
            .map_err(AuthServerError::gas_sponsorship)?;

        Ok(Some(conversion_rate_u256))
    }

    /// Generate the calldata for sponsoring the given match via the gas sponsor
    fn generate_gas_sponsor_calldata(
        &self,
        external_match_resp: &ExternalMatchResponse,
        refund_address: AlloyAddress,
        refund_native_eth: bool,
        sponsorship_amount: AlloyU256,
    ) -> Result<Bytes, AuthServerError> {
        let (nonce, signature) = gen_signed_sponsorship_nonce(
            refund_address,
            sponsorship_amount,
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
                    sponsorship_amount,
                    signature,
                )
            },
            processAtomicMatchSettleWithReceiverCall::SELECTOR => {
                sponsorAtomicMatchSettleWithRefundOptionsCall::from_process_atomic_match_settle_with_receiver_calldata(
                    calldata,
                    refund_address,
                    nonce,
                    refund_native_eth,
                    sponsorship_amount,
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

    /// Get the token & amount spent to sponsor the given settlement
    /// transaction, and the associated transaction hash
    async fn get_sponsorship_amount_and_tx(
        &self,
        settlement_tx: &TypedTransaction,
    ) -> Result<Option<(Address, U256, TxHash)>, AuthServerError> {
        // Parse the nonce from the TX calldata
        let calldata =
            settlement_tx.data().ok_or(AuthServerError::gas_sponsorship("expected calldata"))?;

        let selector = get_selector(calldata)?;

        let nonce = match selector {
            sponsorAtomicMatchSettleWithRefundOptionsCall::SELECTOR => {
                Self::get_nonce_from_sponsored_match_calldata(calldata)?
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
                .topic3(nonce)
                .from_block(self.start_block_num);

        let events = filter.query_with_meta().await.map_err(AuthServerError::gas_sponsorship)?;

        // If no event was found, we assume that gas was not sponsored for this nonce.
        // This could be the case if the gas sponsor was underfunded or paused.
        let sponsorship_event =
            events.last().map(|(event, meta)| (event.token, event.amount, meta.transaction_hash));

        if sponsorship_event.is_none() {
            warn!("No gas sponsorship event found for nonce: {}", nonce);
        }

        Ok(sponsorship_event)
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    pub async fn record_settled_match_sponsorship(
        &self,
        match_bundle: &AtomicMatchApiBundle,
        is_sponsored: bool,
        key: String,
        request_id: String,
    ) -> Result<(), AuthServerError> {
        if is_sponsored
            && let Some((token_addr, amount, tx_hash)) =
                self.get_sponsorship_amount_and_tx(&match_bundle.settlement_tx).await?
        {
            let nominal_price = if token_addr == Address::zero() {
                // The zero address indicates that the gas was sponsored via native ETH.
                let price_f64: f64 = self
                    .price_reporter_client
                    .get_eth_price()
                    .await
                    .map_err(AuthServerError::gas_sponsorship)?;

                let price_bigdecimal = BigDecimal::from_f64(price_f64).ok_or(
                    AuthServerError::gas_sponsorship("failed to convert ETH price to BigDecimal"),
                )?;

                let adjustment = ethers_u256_to_bigdecimal(WEI_IN_ETHER);

                price_bigdecimal / adjustment
            } else {
                // If we did not refund via native ETH, it must have been through the buy-side
                // token. Thus we compute the nominal price of the buy-side
                // token from the match result.
                get_nominal_buy_token_price(match_bundle)?
            };

            let nominal_amount = ethers_u256_to_bigdecimal(amount);
            let value_bigdecimal = nominal_amount * nominal_price;

            let value = value_bigdecimal.to_f64().ok_or(AuthServerError::gas_sponsorship(
                "failed to convert gas sponsorship value to f64",
            ))?;

            self.rate_limiter.record_gas_sponsorship(key, value).await;

            record_gas_sponsorship_metrics(value, tx_hash, request_id);
        }

        Ok(())
    }

    /// Get the nonce from `sponsorAtomicMatchSettleWithRefundOptions` calldata
    fn get_nonce_from_sponsored_match_calldata(calldata: &[u8]) -> Result<U256, AuthServerError> {
        let call = sponsorAtomicMatchSettleWithRefundOptionsCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(U256::from_big_endian(&call.nonce.to_be_bytes_vec()))
    }
}
