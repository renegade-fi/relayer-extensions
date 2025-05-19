//! Direct match endpoint handler

use bytes::Bytes;
use http::{Method, StatusCode};
use renegade_api::http::external_match::AssembleExternalMatchRequest;
use tracing::{instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use crate::{
    error::AuthServerError,
    http_utils::overwrite_response_body,
    server::{api_handlers::log_unsuccessful_relayer_request, Server},
    telemetry::helpers::record_relayer_request_500,
};

impl Server {
    /// Handle an external match request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_match_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let key_description = self.authorize_request(path_str, &query_str, &headers, &body).await?;

        // Direct matches are always shared
        self.check_bundle_rate_limit(key_description.clone(), true /* shared */).await?;

        let (external_match_req, gas_sponsorship_info) = self
            .maybe_apply_gas_sponsorship_to_match_request(
                key_description.clone(),
                &body,
                &query_str,
            )
            .await?;

        let req_body = serde_json::to_vec(&external_match_req).map_err(AuthServerError::serde)?;

        // Send the request to the relayer, potentially sponsoring the gas costs

        let mut resp = self
            .send_admin_request(Method::POST, path_str, headers.clone(), req_body.clone().into())
            .await?;

        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_description.clone(), path_str.to_string());
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(
                &resp,
                &key_description,
                path_str,
                &req_body,
                &headers,
            );
            return Ok(resp);
        }

        let sponsored_match_resp =
            self.maybe_apply_gas_sponsorship_to_match_response(resp.body(), gas_sponsorship_info)?;

        overwrite_response_body(&mut resp, sponsored_match_resp.clone())?;

        // Record the bundle context in the store
        let bundle_id = self
            .write_bundle_context(
                &sponsored_match_resp,
                &headers,
                key_description.clone(),
                true, // shared
            )
            .await?;

        // Watch the bundle for settlement
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.handle_direct_match_bundle_response(
                &key_description,
                &external_match_req,
                &headers,
                &sponsored_match_resp,
                &bundle_id,
            ) {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(resp)
    }
}
