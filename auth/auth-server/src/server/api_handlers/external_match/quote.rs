//! Quote endpoint handler

use bytes::Bytes;
use http::{Method, StatusCode};
use tracing::{error, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use crate::{
    error::AuthServerError,
    http_utils::overwrite_response_body,
    server::{api_handlers::log_unsuccessful_relayer_request, Server},
    telemetry::helpers::record_relayer_request_500,
};

impl Server {
    /// Handle an external quote request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let key_desc = self.authorize_request(path_str, &query_str, &headers, &body).await?;
        self.check_quote_rate_limit(key_desc.clone()).await?;

        // If necessary, ensure that the exact output amount requested in the order is
        // respected by any gas sponsorship applied to the relayer's quote
        let (external_quote_req, gas_sponsorship_info) = self
            .maybe_apply_gas_sponsorship_to_quote_request(key_desc.clone(), &body, &query_str)
            .await?;

        // Send the request to the relayer
        let req_body = serde_json::to_vec(&external_quote_req).map_err(AuthServerError::serde)?;
        let mut resp = self
            .send_admin_request(Method::POST, path_str, headers.clone(), req_body.clone().into())
            .await?;

        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_desc.clone(), path_str.to_string());
        }
        if status == StatusCode::NO_CONTENT {
            self.handle_no_quote_found(&key_desc, &external_quote_req);
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&resp, &key_desc, path_str, &req_body, &headers);
            return Ok(resp);
        }

        let sponsored_quote_response =
            self.maybe_apply_gas_sponsorship_to_quote_response(resp.body(), gas_sponsorship_info)?;
        overwrite_response_body(&mut resp, sponsored_quote_response.clone())?;

        let server_clone = self.clone();
        tokio::spawn(async move {
            // Cache the gas sponsorship info for the quote in Redis if it exists
            if let Err(e) =
                server_clone.cache_quote_gas_sponsorship_info(&sponsored_quote_response).await
            {
                error!("Error caching quote gas sponsorship info: {e}");
            }

            // Log the quote response & emit metrics
            if let Err(e) = server_clone.handle_quote_response(
                key_desc,
                &external_quote_req,
                &headers,
                &sponsored_quote_response,
            ) {
                warn!("Error handling quote: {e}");
            }
        });

        Ok(resp)
    }
}
