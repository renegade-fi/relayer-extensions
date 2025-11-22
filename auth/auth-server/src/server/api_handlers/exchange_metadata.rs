//! Exchange metadata endpoint handler

use auth_server_api::exchange_metadata::ExchangeMetadataResponse;
use bytes::Bytes;
use http::{HeaderMap, Method};
use renegade_api::http::price_report::{GET_SUPPORTED_TOKENS_ROUTE, GetSupportedTokensResponse};
use tracing::instrument;
use warp::reject::Rejection;
use warp::reply::Json;

use super::Server;
use crate::error::AuthServerError;

impl Server {
    /// Handle a GET request to the /v0/exchange-metadata endpoint
    ///
    /// This endpoint proxies the request to the relayer to get exchange
    /// metadata including chain ID, settlement contract address, and
    /// supported tokens.
    #[instrument(skip(self, path, headers))]
    pub async fn handle_exchange_metadata_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
    ) -> Result<Json, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        self.authorize_request(path_str, "" /* query_str */, &headers, &[] /* body */).await?;

        // Fetch supported tokens
        let supported_tokens = self.get_supported_tokens().await?;

        // Create the response
        let response = ExchangeMetadataResponse {
            chain_id: self.chain.chain_id(),
            settlement_contract_address: self.gas_sponsor_address.to_string(),
            supported_tokens: supported_tokens.tokens,
        };
        Ok(warp::reply::json(&response))
    }

    /// Get supported tokens from the relayer
    async fn get_supported_tokens(&self) -> Result<GetSupportedTokensResponse, AuthServerError> {
        let resp = self
            .send_admin_request(
                Method::GET,
                GET_SUPPORTED_TOKENS_ROUTE,
                HeaderMap::new(),
                Bytes::new(),
            )
            .await?;
        if !resp.status().is_success() {
            return Err(AuthServerError::custom("Failed to get supported tokens"));
        }

        let body = serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;
        Ok(body)
    }
}
