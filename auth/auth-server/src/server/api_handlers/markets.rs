//! Market endpoint handlers

use bytes::Bytes;
use futures_util::future;
use http::{HeaderMap, Method, StatusCode};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_external_api::{
    http::market::{
        GET_MARKET_DEPTH_BY_MINT_ROUTE, GET_MARKETS_DEPTH_ROUTE, GetMarketDepthByMintResponse,
        GetMarketDepthsResponse,
    },
    types::market::MarketDepth,
};
use tokio::task::JoinHandle;
use tracing::instrument;
use uuid::Uuid;
use warp::reject::Rejection;

use super::{Server, log_unsuccessful_relayer_request};
use crate::{
    error::AuthServerError,
    http_utils::request_response::{overwrite_response_body, should_stringify_numbers},
    server::api_handlers::external_match::BytesResponse,
    telemetry::helpers::record_relayer_request_500,
};

impl Server {
    /// Return market depth for a specific mint with user-specific relayer fees
    #[instrument(skip(self, path, headers))]
    pub async fn handle_market_depth_by_mint_request(
        &self,
        mint: String,
        path: warp::path::FullPath,
        headers: HeaderMap,
    ) -> Result<BytesResponse, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let (key_desc, key_id) = self
            .authorize_request(path_str, "" /* query_str */, &headers, &[] /* body */)
            .await?;

        // Check if stringification is requested
        let should_stringify = should_stringify_numbers(&headers);

        // Construct the relayer path
        let relayer_path = GET_MARKET_DEPTH_BY_MINT_ROUTE.replace(":mint", &mint);

        // Forward the request, return errors immediately if the request fails
        let mut resp =
            self.handle_market_request_internal(&relayer_path, &key_desc, headers.clone()).await?;

        if !resp.status().is_success() {
            return Ok(resp);
        }

        // Deserialize the response body and replace the fee data
        let mut response: GetMarketDepthByMintResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;

        self.replace_external_match_fee_rate(key_id, &mut response.market_depth).await?;
        overwrite_response_body(&mut resp, response, should_stringify)?;

        Ok(resp)
    }

    /// Return market depth for all markets with user-specific relayer fees
    #[instrument(skip(self, path, headers))]
    pub async fn handle_all_markets_depth_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
    ) -> Result<BytesResponse, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let (key_desc, key_id) = self
            .authorize_request(path_str, "" /* query_str */, &headers, &[] /* body */)
            .await?;

        // Check if stringification is requested before headers are moved
        let should_stringify = should_stringify_numbers(&headers);

        // Forward the request
        let mut resp = self
            .handle_market_request_internal(GET_MARKETS_DEPTH_ROUTE, &key_desc, headers)
            .await?;

        if !resp.status().is_success() {
            return Ok(resp);
        }

        // Deserialize the response body and replace the fee data
        let mut body: GetMarketDepthsResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;

        // Update all markets' external match relayer fee rates concurrently
        let mut futures = Vec::<JoinHandle<Result<MarketDepth, AuthServerError>>>::new();
        for market_depth in body.market_depths.iter().cloned() {
            let self_clone = self.clone();
            let jh = tokio::spawn(async move {
                let mut market_depth = market_depth;
                self_clone.replace_external_match_fee_rate(key_id, &mut market_depth).await?;
                Ok(market_depth)
            });

            futures.push(jh);
        }

        let market_depths: Vec<MarketDepth> = future::join_all(futures)
            .await
            .into_iter()
            .map(|result| {
                result.map_err(|e| AuthServerError::custom(format!("Join error: {e}")))?
            })
            .collect::<Result<Vec<_>, _>>()?;

        body.market_depths = market_depths;
        overwrite_response_body(&mut resp, body, should_stringify)?;

        Ok(resp)
    }

    // -----------
    // | Helpers |
    // -----------

    /// Proxy GET requests to /v2/markets/* endpoints to the relayer
    #[instrument(skip(self, path, headers))]
    async fn handle_market_request_internal(
        &self,
        path: &str,
        key_desc: &str,
        headers: HeaderMap,
    ) -> Result<BytesResponse, Rejection> {
        // Send the request to the relayer
        let resp =
            self.send_admin_request(Method::GET, path, headers.clone(), Bytes::new()).await?;

        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_desc.to_string(), path.to_string());
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&resp, key_desc, path, &headers);
            return Ok(resp);
        }
        Ok(resp)
    }

    /// Replace the external match relayer fee rate for a given market depth
    async fn replace_external_match_fee_rate(
        &self,
        user_id: Uuid,
        market_depth: &mut MarketDepth,
    ) -> Result<(), AuthServerError> {
        let ticker = market_depth.market.base.symbol.clone();
        let user_fee = self.get_user_fee(user_id, ticker).await?;
        market_depth.market.external_match_fee_rates.relayer_fee_rate =
            FixedPoint::from_f64_round_down(user_fee);

        Ok(())
    }
}
