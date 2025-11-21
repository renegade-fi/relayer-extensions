//! Orderbook endpoint handlers
use bytes::Bytes;
use futures_util::future;
use http::{HeaderMap, Method, StatusCode};
use renegade_api::http::order_book::{GetDepthForAllPairsResponse, PriceAndDepth};
use renegade_common::types::token::Token;
use tokio::task::JoinHandle;
use tracing::instrument;
use uuid::Uuid;
use warp::reject::Rejection;

use super::{Server, log_unsuccessful_relayer_request};
use crate::error::AuthServerError;
use crate::http_utils::request_response::overwrite_response_body;
use crate::server::api_handlers::external_match::BytesResponse;
use crate::telemetry::helpers::record_relayer_request_500;

impl Server {
    /// Handle a GET request to the /v0/order_book/:mint endpoint
    ///
    /// We deserialize the response from the relayer and replace the relayer fee
    /// with the fee that the auth server will use for the given user.
    #[instrument(skip(self, path, headers))]
    pub async fn handle_order_book_request_with_mint(
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

        // Forward the request, return errors immediately if the request fails
        let mut resp =
            self.handle_order_book_request_internal(path_str, &key_desc, headers).await?;
        if !resp.status().is_success() {
            return Ok(resp);
        }

        // Deserialize the response body and replace the fee data
        let mut depth: PriceAndDepth =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;
        self.replace_relayer_fee_rate(key_id, &mut depth).await?;
        overwrite_response_body(&mut resp, depth, false /* stringify */)?;

        Ok(resp)
    }

    /// Handle a GET request to the /v0/order_book/depth endpoint
    ///
    /// We deserialize the response from the relayer and replace the relayer fee
    /// on each pair's price and depth data with the fee that the auth
    /// server will use for the given user.
    pub async fn handle_all_pairs_order_book_depth_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
    ) -> Result<BytesResponse, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let (key_desc, key_id) = self
            .authorize_request(path_str, "" /* query_str */, &headers, &[] /* body */)
            .await?;

        // Forward the request
        let mut resp =
            self.handle_order_book_request_internal(path_str, &key_desc, headers).await?;
        if !resp.status().is_success() {
            return Ok(resp);
        }

        // Deserialize the response body and replace the fee data
        let mut body: GetDepthForAllPairsResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;

        // Update all pairs' relayer fee rates concurrently
        let mut futures = Vec::<JoinHandle<Result<PriceAndDepth, AuthServerError>>>::new();
        for pair in body.pairs.iter().cloned() {
            let self_clone = self.clone();
            let jh = tokio::spawn(async move {
                let mut pair = pair;
                self_clone.replace_relayer_fee_rate(key_id, &mut pair).await?;
                Ok(pair)
            });

            futures.push(jh);
        }

        let pairs: Vec<PriceAndDepth> = future::join_all(futures)
            .await
            .into_iter()
            .map(|result| {
                result.map_err(|e| AuthServerError::custom(format!("Join error: {e}")))?
            })
            .collect::<Result<Vec<_>, _>>()?;

        body.pairs = pairs;
        overwrite_response_body(&mut resp, body, false /* stringify */)?;

        Ok(resp)
    }

    // -----------
    // | Helpers |
    // -----------

    /// Proxy GET requests to /v0/order_book/* endpoints to the relayer
    #[instrument(skip(self, path, headers))]
    async fn handle_order_book_request_internal(
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

    /// Replace the relayer fee rate for a given `PriceAndDepth` instance
    async fn replace_relayer_fee_rate(
        &self,
        user_id: Uuid,
        price_and_depth: &mut PriceAndDepth,
    ) -> Result<(), AuthServerError> {
        let token = Token::from_addr(&price_and_depth.address);
        let ticker = token.get_ticker().ok_or(AuthServerError::bad_request("Invalid token"))?;
        let user_fee = self.get_user_fee(user_id, ticker).await?;
        price_and_depth.fee_rates.relayer_fee_rate = user_fee;

        Ok(())
    }
}
