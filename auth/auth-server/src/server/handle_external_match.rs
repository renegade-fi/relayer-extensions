//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::Address;
use alloy_sol_types::{sol, SolCall};
use auth_server_api::{ExternalMatchResponse, GasSponsorshipQueryParams};
use bytes::Bytes;
use ethers::contract::abigen;
use ethers::types::{transaction::eip2718::TypedTransaction, TxHash, U256};
use ethers::utils::format_ether;
use http::header::CONTENT_LENGTH;
use http::{Method, Response, StatusCode};
use renegade_arbitrum_client::abi::{
    processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall,
};
use tracing::{info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, ExternalMatchRequest,
    ExternalMatchResponse as RelayerExternalMatchResponse, ExternalOrder, ExternalQuoteResponse,
};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_common::types::{token::Token, TimestampedPrice};

use super::helpers::{gen_signed_sponsorship_nonce, get_selector};
use super::Server;
use crate::error::AuthServerError;
use crate::telemetry::helpers::{calculate_implied_price, record_gas_sponsorship_metrics};
use crate::telemetry::labels::GAS_SPONSORED_METRIC_TAG;
use crate::telemetry::{
    helpers::{
        await_settlement, record_endpoint_metrics, record_external_match_metrics, record_fill_ratio,
    },
    labels::{
        DECIMAL_CORRECTION_FIXED_METRIC_TAG, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT,
        KEY_DESCRIPTION_METRIC_TAG, REQUEST_ID_METRIC_TAG,
    },
};

// -------------
// | Constants |
// -------------

/// The gas estimation to use if fetching a gas estimation fails
/// From https://github.com/renegade-fi/renegade/blob/main/workers/api-server/src/http/external_match.rs/#L62
pub const DEFAULT_GAS_ESTIMATION: u64 = 4_000_000; // 4m

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
        event AmountSponsored(uint256 indexed amount, uint256 indexed nonce)
    ]"#
);

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Handle an external quote request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let key_desc = self.authorize_request(path.as_str(), &headers, &body).await?;
        self.check_quote_rate_limit(key_desc.clone()).await?;

        // Send the request to the relayer
        let resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.handle_quote_response(key_desc, &body, &resp_clone) {
                warn!("Error handling quote: {e}");
            }
        });

        Ok(resp)
    }

    /// Handle an external quote-assembly request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_quote_assembly_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_params: GasSponsorshipQueryParams,
    ) -> Result<impl Reply, Rejection> {
        // Serialize the path + query params for auth
        let query_str = serde_urlencoded::to_string(&query_params).unwrap();
        let auth_path = if query_str.is_empty() {
            path.as_str().to_string()
        } else {
            format!("{}?{}", path.as_str(), query_str)
        };

        // Authorize the request
        let key_desc = self.authorize_request(&auth_path, &headers, &body).await?;
        self.check_bundle_rate_limit(key_desc.clone()).await?;

        let is_sponsored = query_params.use_gas_sponsorship.unwrap_or(false)
            && self.check_gas_sponsorship_rate_limit(key_desc.clone()).await;

        let refund_address = query_params.get_refund_address().map_err(AuthServerError::serde)?;

        // Send the request to the relayer, potentially sponsoring the gas costs

        let mut resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        let status = resp.status();
        if status != StatusCode::OK {
            warn!("Non-200 response from relayer: {}", status);
            return Ok(resp);
        }

        self.mutate_response_for_gas_sponsorship(&mut resp, is_sponsored, refund_address)?;

        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_quote_assembly_bundle_response(key_desc, &body, &resp_clone)
                .await
            {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(resp)
    }

    /// Handle an external match request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_match_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_params: GasSponsorshipQueryParams,
    ) -> Result<impl Reply, Rejection> {
        // Serialize the path + query params for auth
        let query_str = serde_urlencoded::to_string(&query_params).unwrap();
        let auth_path = if query_str.is_empty() {
            path.as_str().to_string()
        } else {
            format!("{}?{}", path.as_str(), query_str)
        };

        // Authorize the request
        let key_description = self.authorize_request(&auth_path, &headers, &body).await?;
        self.check_bundle_rate_limit(key_description.clone()).await?;

        let is_sponsored = query_params.use_gas_sponsorship.unwrap_or(false)
            && self.check_gas_sponsorship_rate_limit(key_description.clone()).await;

        let refund_address = query_params.get_refund_address().map_err(AuthServerError::serde)?;

        // Send the request to the relayer, potentially sponsoring the gas costs

        let mut resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        let status = resp.status();
        if status != StatusCode::OK {
            warn!("Non-200 response from relayer: {}", status);
            return Ok(resp);
        }

        self.mutate_response_for_gas_sponsorship(&mut resp, is_sponsored, refund_address)?;

        // Watch the bundle for settlement
        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_direct_match_bundle_response(key_description, &body, &resp_clone)
                .await
            {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(resp)
    }

    // --- Bundle Tracking --- //

    /// Handle a bundle response from a quote assembly request
    async fn handle_quote_assembly_bundle_response(
        &self,
        key: String,
        req: &[u8],
        resp: &[u8],
    ) -> Result<(), AuthServerError> {
        let req: AssembleExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;

        let original_order = &req.signed_quote.quote.order;
        let updated_order = req.updated_order.as_ref().unwrap_or(original_order);

        let request_id = uuid::Uuid::new_v4().to_string();
        if req.updated_order.is_some() {
            self.log_updated_order(&key, original_order, updated_order, &request_id);
        }

        self.handle_bundle_response(key, updated_order.clone(), resp, Some(request_id)).await
    }

    /// Handle a bundle response from a direct match request
    async fn handle_direct_match_bundle_response(
        &self,
        key: String,
        req: &[u8],
        resp: &[u8],
    ) -> Result<(), AuthServerError> {
        let req: ExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;
        let order = req.external_order;
        self.handle_bundle_response(key, order, resp, None).await
    }

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    async fn handle_bundle_response(
        &self,
        key: String,
        order: ExternalOrder,
        resp: &[u8],
        request_id: Option<String>,
    ) -> Result<(), AuthServerError> {
        // Log the bundle
        let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.log_bundle(&order, resp, &key, &request_id)?;

        // Deserialize the response
        let match_resp: ExternalMatchResponse =
            serde_json::from_slice(resp).map_err(AuthServerError::serde)?;

        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key.clone()),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id.clone()),
            (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), match_resp.is_sponsored.to_string()),
        ];

        // Record quote comparisons before settlement, if enabled
        if let Some(quote_metrics) = &self.quote_metrics {
            quote_metrics
                .record_quote_comparison(&match_resp.match_bundle, labels.as_slice())
                .await;
        }

        // If the bundle settles, increase the API user's a rate limit token balance
        let did_settle = await_settlement(&match_resp.match_bundle, &self.arbitrum_client).await?;
        if did_settle {
            self.add_bundle_rate_limit_token(key.clone()).await;
            self.record_settled_match_sponsorship(&match_resp, key, request_id).await?;
        }

        // Record metrics
        record_external_match_metrics(&order, match_resp.match_bundle, &labels, did_settle).await?;

        Ok(())
    }

    // --- Logging --- //

    /// Log a quote
    fn log_quote(&self, quote_bytes: &[u8]) -> Result<(), AuthServerError> {
        let resp = serde_json::from_slice::<ExternalQuoteResponse>(quote_bytes)
            .map_err(AuthServerError::serde)?;

        let match_result = resp.signed_quote.match_result();
        let is_buy = match_result.direction;
        let recv = resp.signed_quote.receive_amount();
        let send = resp.signed_quote.send_amount();
        info!(
            "Sending quote(is_buy: {is_buy}, receive: {} ({}), send: {} ({})) to client",
            recv.amount, recv.mint, send.amount, send.mint
        );

        Ok(())
    }

    /// Log an updated order
    fn log_updated_order(
        &self,
        key: &str,
        original_order: &ExternalOrder,
        updated_order: &ExternalOrder,
        request_id: &str,
    ) {
        let original_base_amount = original_order.base_amount;
        let updated_base_amount = updated_order.base_amount;
        let original_quote_amount = original_order.quote_amount;
        let updated_quote_amount = updated_order.quote_amount;
        info!(
            key_description = key,
            request_id = request_id,
            "Quote updated(original_base_amount: {}, updated_base_amount: {}, original_quote_amount: {}, updated_quote_amount: {})",
            original_base_amount, updated_base_amount, original_quote_amount, updated_quote_amount
        );
    }

    /// Log the bundle parameters
    fn log_bundle(
        &self,
        order: &ExternalOrder,
        bundle_bytes: &[u8],
        key_description: &str,
        request_id: &str,
    ) -> Result<(), AuthServerError> {
        let resp = serde_json::from_slice::<ExternalMatchResponse>(bundle_bytes)
            .map_err(AuthServerError::serde)?;

        // Get the decimal-corrected price
        let price = calculate_implied_price(&resp.match_bundle, true /* decimal_correct */)?;
        let price_fixed = FixedPoint::from_f64_round_down(price);

        let match_result = resp.match_bundle.match_result;
        let is_buy = match_result.direction;
        let recv = resp.match_bundle.receive;
        let send = resp.match_bundle.send;

        // Get the base fill ratio
        let requested_base_amount = order.get_base_amount(price_fixed);
        let response_base_amount = match_result.base_amount;
        let base_fill_ratio = response_base_amount as f64 / requested_base_amount as f64;

        // Get the quote fill ratio
        let requested_quote_amount = order.get_quote_amount(price_fixed);
        let response_quote_amount = match_result.quote_amount;
        let quote_fill_ratio = response_quote_amount as f64 / requested_quote_amount as f64;

        info!(
            requested_base_amount = requested_base_amount,
            response_base_amount = response_base_amount,
            requested_quote_amount = requested_quote_amount,
            response_quote_amount = response_quote_amount,
            base_fill_ratio = base_fill_ratio,
            quote_fill_ratio = quote_fill_ratio,
            key_description = key_description,
            request_id = request_id,
            "Sending bundle(is_buy: {}, recv: {} ({}), send: {} ({})) to client",
            is_buy,
            recv.amount,
            recv.mint,
            send.amount,
            send.mint
        );

        Ok(())
    }

    /// Handle a quote response
    fn handle_quote_response(
        &self,
        key: String,
        req: &[u8],
        resp: &[u8],
    ) -> Result<(), AuthServerError> {
        // Log the quote parameters
        self.log_quote(resp)?;

        // Only proceed with metrics recording if sampled
        if !self.should_sample_metrics() {
            return Ok(());
        }

        // Parse request and response for metrics
        let req: ExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;
        let quote_resp: ExternalQuoteResponse =
            serde_json::from_slice(resp).map_err(AuthServerError::serde)?;

        // Get the decimal-corrected price
        let price: TimestampedPrice = quote_resp.signed_quote.quote.price.clone().into();

        // Calculate requested and matched quote amounts
        let requested_quote_amount =
            req.external_order.get_quote_amount(FixedPoint::from_f64_round_down(price.price));
        let matched_quote_amount = quote_resp.signed_quote.quote.match_result.quote_amount;

        // Record fill ratio metric
        let request_id = uuid::Uuid::new_v4();
        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id.to_string()),
            (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
        ];
        record_fill_ratio(requested_quote_amount, matched_quote_amount, &labels)?;

        // Record endpoint metrics
        let base_token = Token::from_addr_biguint(&req.external_order.base_mint);
        record_endpoint_metrics(&base_token.addr, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT, &labels);

        Ok(())
    }

    /// Mutate a quote assembly response to invoke gas sponsorship
    fn mutate_response_for_gas_sponsorship(
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
                .event::<AmountSponsoredFilter>()
                .address(self.gas_sponsor_address.into())
                .topic2(nonce)
                .from_block(self.start_block_num);

        let events = filter.query_with_meta().await.map_err(AuthServerError::gas_sponsorship)?;

        // If no event was found, we assume that gas was not sponsored for this nonce.
        // This could be the case if the gas sponsor was underfunded or paused.
        let amount_sponsored_with_tx =
            events.last().map(|(event, meta)| (event.amount, meta.transaction_hash));

        Ok(amount_sponsored_with_tx)
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    async fn record_settled_match_sponsorship(
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

            self.record_gas_sponsorship_rate_limit(key, gas_sponsorship_value).await?;

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
