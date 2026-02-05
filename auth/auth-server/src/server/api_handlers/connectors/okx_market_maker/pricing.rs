//! Handlers for the pricing endpoint

use bytes::Bytes;
use http::Method;
use renegade_external_api::http::order_book::{GET_DEPTH_FOR_ALL_PAIRS_ROUTE, GetDepthForAllPairsResponse};
use renegade_common::types::token::Token;
use warp::{reject::Rejection, reply::Json};

use crate::{
    error::AuthServerError,
    server::{
        Server,
        api_handlers::connectors::okx_market_maker::{
            api_types::{LevelDataEntry, OkxPricingData, OkxPricingResponse},
            helpers::parse_chain_id,
        },
    },
};

impl Server {
    /// Handle the pricing endpoint
    pub async fn handle_pricing_request(
        &self,
        path: warp::path::FullPath,
        query_str: String,
        headers: warp::hyper::HeaderMap,
    ) -> Result<Json, Rejection> {
        // Validate the request
        self.validate_pricing_request(path, query_str, headers.clone()).await?;

        // Fetch order book data for all pairs
        let resp = self
            .send_admin_request(
                Method::GET,
                GET_DEPTH_FOR_ALL_PAIRS_ROUTE,
                headers.clone(),
                Bytes::new(),
            )
            .await?;

        // Deserialize the response
        let resp: GetDepthForAllPairsResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;
        let pricing_response = Self::transform_depth_to_pricing(self.chain.chain_id(), &resp)?;

        Ok(warp::reply::json(&pricing_response))
    }

    /// Validate a pricing request
    async fn validate_pricing_request(
        &self,
        path: warp::path::FullPath,
        query_str: String,
        headers: warp::hyper::HeaderMap,
    ) -> Result<(), AuthServerError> {
        // Check that the chain ID matches this server's chain
        let chain_id = parse_chain_id(&query_str)?;
        let my_chain_id = self.chain.chain_id();
        if chain_id != my_chain_id {
            return Err(AuthServerError::bad_request(format!(
                "Chain ID mismatch: expected {my_chain_id}, got {chain_id}"
            )));
        }

        // Authorize the request
        self.authorize_request(
            path.as_str(),
            &query_str,
            &headers, // headers
            &[],      // body
        )
        .await?;

        Ok(())
    }

    /// Transform a relayer all pairs depth response into an OKX pricing
    /// response
    fn transform_depth_to_pricing(
        chain_id: u64,
        depth_response: &GetDepthForAllPairsResponse,
    ) -> Result<OkxPricingResponse, AuthServerError> {
        let usdc = Token::usdc().get_addr();
        let mut entries = Vec::new();

        for pair in depth_response.pairs.iter() {
            let base_token = &pair.address;
            let quote_token = &usdc;

            // Buy side
            // Maker buys the base token with the quote token, so the taker address is the
            // base token address
            entries.push(LevelDataEntry {
                taker_token_address: base_token.clone(),
                maker_token_address: quote_token.clone(),
                levels: vec![(pair.buy.total_quantity.to_string(), pair.price.to_string())],
            });

            // Sell side
            // Maker sells the base token with the quote token, so the taker address is the
            // quote token address
            // The price is expected to be in units of maker token per units of taker token,
            // Renegade prices are in quote / base so we need to invert it
            let sell_price = 1.0 / pair.price;
            entries.push(LevelDataEntry {
                taker_token_address: quote_token.clone(),
                maker_token_address: base_token.clone(),
                levels: vec![(pair.sell.total_quantity.to_string(), sell_price.to_string())],
            });
        }

        Ok(OkxPricingResponse {
            code: "0".to_string(),
            msg: "Success".to_string(),
            data: OkxPricingData { chain_index: chain_id.to_string(), level_data: entries },
        })
    }
}
