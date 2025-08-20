//! Handlers for external match endpoints

mod assemble_malleable_quote;
mod assemble_quote;
mod direct_match;
mod quote;

use alloy_primitives::U256;
use auth_server_api::GasSponsorshipInfo;
use bytes::Bytes;
use http::{HeaderMap, Method, Response, StatusCode};
use num_bigint::BigUint;
use renegade_common::types::token::Token;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::AuthServerError,
    http_utils::{
        request_response::should_stringify_numbers, stringify_formatter::json_deserialize,
    },
    server::{Server, api_handlers::ticker_from_biguint},
    telemetry::helpers::record_relayer_request_500,
};
pub(crate) use assemble_malleable_quote::SponsoredAssembleMalleableQuoteResponseCtx;

use super::{get_sdk_version, log_unsuccessful_relayer_request};

/// A bytes body response type
pub type BytesResponse = Response<Bytes>;

// --------------------
// | Request Contexts |
// --------------------

// --- Request Context --- //

/// Context for an external match request
///
/// This struct handles the common logic for request context helpers used in
#[derive(Debug, Clone)]
pub struct RequestContext<Req: Serialize + for<'de> Deserialize<'de>> {
    /// The path of the request
    pub path: String,
    /// The query string of the request
    pub query_str: String,
    /// The API user description for the request
    ///
    /// Derived from the API key
    pub user: String,
    /// The API key id
    pub key_id: Uuid,
    /// The version of the SDK used to make the request
    pub sdk_version: String,
    /// The headers of the request
    pub headers: HeaderMap,
    /// The body of the request
    pub body: Req,
    /// The gas sponsorship info for the request
    pub sponsorship_info: Option<GasSponsorshipInfo>,
    /// Unique ID for this request
    pub request_id: Uuid,
}

impl<Req: Serialize + for<'de> Deserialize<'de>> RequestContext<Req> {
    /// Get the user description for the request
    pub fn user(&self) -> String {
        self.user.to_string()
    }

    /// Get the API key id for the request
    pub fn key_id(&self) -> Uuid {
        self.key_id
    }

    /// Get a reference to the query string
    pub fn query(&self) -> String {
        self.query_str.to_string()
    }

    /// Get a reference to the path
    pub fn path(&self) -> String {
        self.path.to_string()
    }

    /// Get a mutable reference to the request body
    pub fn body_mut(&mut self) -> &mut Req {
        &mut self.body
    }

    /// Get the json encoded body bytes
    pub fn body_bytes(&self) -> Result<Vec<u8>, AuthServerError> {
        serde_json::to_vec(&self.body).map_err(AuthServerError::serde)
    }

    /// Attach gas sponsorship info to the request context
    pub fn set_sponsorship_info(&mut self, info: Option<GasSponsorshipInfo>) {
        self.sponsorship_info = info;
    }
}

/// A trait used to define access patterns on different request types
#[allow(unused)]
pub trait ExternalMatchRequestType: Serialize + for<'de> Deserialize<'de> {
    /// Get the base token for the request
    fn base_mint(&self) -> &BigUint;
    /// Get the base ticker for the request
    fn base_ticker(&self) -> Result<String, AuthServerError> {
        ticker_from_biguint(self.base_mint())
    }

    /// Get the quote token for the request
    fn quote_mint(&self) -> &BigUint;
    /// Get the quote ticker for the request
    fn quote_ticker(&self) -> Result<String, AuthServerError> {
        ticker_from_biguint(self.quote_mint())
    }

    /// Set the fee for the request
    fn set_fee(&mut self, fee: f64);
}

// --- Response Context --- //

/// Context for an external match response
#[derive(Debug, Clone)]
pub struct ResponseContext<
    Req: Serialize + for<'de> Deserialize<'de>,
    Resp: Serialize + for<'de> Deserialize<'de>,
> {
    /// The path of the request
    pub path: String,
    /// The query string of the request
    pub query_str: String,
    /// Derived from the API key
    pub user: String,
    /// The version of the SDK used to make the request
    pub sdk_version: String,
    /// The headers of the request
    pub headers: HeaderMap,
    /// A copy of the request
    pub request: Req,
    /// A copy of the raw response
    pub status: StatusCode,
    /// The deserialized response body
    ///
    /// May be `None` if the relayer returned a non-200 status code
    pub response: Option<Resp>,
    /// The gas sponsorship info for the request, along with a nonce
    pub sponsorship_info_with_nonce: Option<(GasSponsorshipInfo, U256)>,
    /// Unique ID for this request-response flow
    pub request_id: Uuid,
}

impl<
    Req: Serialize + for<'de> Deserialize<'de>,
    Resp: Serialize + for<'de> Deserialize<'de> + Clone,
> ResponseContext<Req, Resp>
{
    /// Create a response context from a response and request context
    pub fn from_response(
        status: StatusCode,
        body: Option<Resp>,
        request: RequestContext<Req>,
    ) -> Result<ResponseContext<Req, Resp>, AuthServerError> {
        // Generate a random nonce for the sponsorship info, if it's present.
        // In the case of quote responses, this will be ignored.
        let sponsorship_info_with_nonce =
            request.sponsorship_info.map(|info| (info, U256::random()));

        Ok(ResponseContext {
            path: request.path,
            query_str: request.query_str,
            user: request.user,
            sdk_version: request.sdk_version,
            headers: request.headers,
            request: request.body,
            status,
            response: body,
            sponsorship_info_with_nonce,
            request_id: request.request_id,
        })
    }

    /// Whether the relayer returned a 200 status code
    pub fn is_success(&self) -> bool {
        self.status == StatusCode::OK
    }

    /// Get the status code of the response
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get a reference to the user description
    pub fn user(&self) -> String {
        self.user.to_string()
    }

    /// Get a reference to the request
    pub fn request(&self) -> &Req {
        &self.request
    }

    /// Get a reference to the response body unwrapping its optional
    ///
    /// Panics if the response body is `None`
    pub fn response(&self) -> Resp {
        self.response.as_ref().unwrap().clone()
    }

    /// Get a reference to the gas sponsorship info
    pub fn sponsorship_info(&self) -> Option<GasSponsorshipInfo> {
        self.sponsorship_info_with_nonce.as_ref().map(|(info, _)| info.clone())
    }

    /// Get a reference to the gas sponsorship info & nonce
    pub fn sponsorship_info_with_nonce(&self) -> Option<(GasSponsorshipInfo, U256)> {
        self.sponsorship_info_with_nonce.clone()
    }

    /// Whether the response body should stringify numeric types
    ///
    /// This is encoded in the accept header by setting `number=string`
    pub fn should_stringify_body(&self) -> bool {
        should_stringify_numbers(&self.headers)
    }
}

// ---------------------------
// | Request Context Helpers |
// ---------------------------

impl Server {
    /// Set the fee for an external match request
    pub async fn set_relayer_fee<Req>(
        &self,
        ctx: &mut RequestContext<Req>,
    ) -> Result<(), AuthServerError>
    where
        Req: ExternalMatchRequestType,
    {
        let user_id = ctx.key_id();
        let ticker = ctx.body.base_ticker()?;
        let user_fee = self.get_user_fee(user_id, ticker).await?;
        ctx.body_mut().set_fee(user_fee);

        Ok(())
    }

    /// Build request context for an external match related request, before the
    /// request is proxied to the relayer
    #[instrument(skip_all)]
    pub async fn preprocess_request<Req>(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<RequestContext<Req>, AuthServerError>
    where
        Req: ExternalMatchRequestType,
    {
        // Authorize the request
        let path = path.as_str().to_string();
        let (key_desc, key_id) = self.authorize_request(&path, &query_str, &headers, &body).await?;
        let sdk_version = get_sdk_version(&headers);

        // Deserialize the request body, then build the context
        let should_stringify = should_stringify_numbers(&headers);
        let body: Req = json_deserialize(&body, should_stringify)?;
        self.validate_request_body(&body)?;

        let mut ctx = RequestContext {
            path,
            query_str,
            sdk_version,
            headers,
            user: key_desc,
            key_id,
            body,
            sponsorship_info: None,
            request_id: Uuid::new_v4(),
        };

        // Set the relayer fee
        self.set_relayer_fee(&mut ctx).await?;
        Ok(ctx)
    }

    /// Validate the request body
    fn validate_request_body<Req>(&self, body: &Req) -> Result<(), AuthServerError>
    where
        Req: ExternalMatchRequestType,
    {
        // Check that the base and quote tokens are valid
        let base = Token::from_addr_biguint(body.base_mint());
        let quote = Token::from_addr_biguint(body.quote_mint());

        let base_valid = base.is_named() || base.is_native_asset();
        if !base_valid {
            let base_addr = base.get_addr();
            return Err(AuthServerError::bad_request(format!("Invalid base token: {base_addr}")));
        }

        if quote != Token::usdc() {
            let quote_addr = quote.get_addr();
            return Err(AuthServerError::bad_request(format!(
                "Quote token must be USDC, got {quote_addr}"
            )));
        }

        Ok(())
    }

    /// Forward a request context to the relayer's admin API, returning the
    /// associated response context
    ///
    /// Returns the raw response from the relayer as well as the response
    /// context generated from the relayer's response
    #[instrument(skip_all)]
    pub async fn forward_request<Req, Resp>(
        &self,
        ctx: RequestContext<Req>,
    ) -> Result<(Response<Bytes>, ResponseContext<Req, Resp>), AuthServerError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
        Resp: Serialize + for<'de> Deserialize<'de> + Clone,
    {
        let body_bytes = ctx.body_bytes()?;
        let resp = self
            .send_admin_request(Method::POST, &ctx.path, ctx.headers.clone(), body_bytes.into())
            .await?;

        // Handle the status codes
        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(ctx.user(), ctx.path());
        } else if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&resp, &ctx.user(), &ctx.path(), &ctx.headers);
        }

        // Deserialize the response body
        let response = if status == StatusCode::OK {
            let body = serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;
            Some(body)
        } else {
            None
        };

        let ctx = ResponseContext::from_response(status, response, ctx)?;
        Ok((resp, ctx))
    }
}
