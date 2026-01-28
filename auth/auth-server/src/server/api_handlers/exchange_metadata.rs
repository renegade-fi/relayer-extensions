//! Exchange metadata endpoint handler

use bytes::Bytes;
use http::{HeaderMap, Method};
use renegade_external_api::{
    http::metadata::GET_EXCHANGE_METADATA_ROUTE, types::ExchangeMetadataResponse,
};
use tracing::instrument;
use warp::{reject::Rejection, reply::Json};

use crate::error::AuthServerError;

use super::Server;

impl Server {
    /// Handle a request for exchange metadata
    #[instrument(skip(self, path, headers))]
    pub async fn handle_exchange_metadata_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
    ) -> Result<Json, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        self.authorize_request(path_str, "" /* query_str */, &headers, &[] /* body */).await?;

        // Proxy the request to the relayer
        let resp = self
            .send_admin_request(
                Method::GET,
                GET_EXCHANGE_METADATA_ROUTE,
                HeaderMap::new(),
                Bytes::new(),
            )
            .await?;

        // Parse the response and overwrite the settlement contract address
        let mut response: ExchangeMetadataResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;

        response.settlement_contract_address = self.gas_sponsor_address;

        Ok(warp::reply::json(&response))
    }
}
