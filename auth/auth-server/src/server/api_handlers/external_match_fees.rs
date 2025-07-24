//! Handles admin external match fee requests

use bytes::Bytes;
use http::HeaderMap;
use renegade_common::types::token::Token;
use tracing::instrument;
use warp::{filters::path::FullPath, reject::Rejection, reply::Json};

use auth_server_api::fee_management::{
    AssetDefaultFeeEntry, GetAllFeesResponse, RemoveAssetDefaultFeeRequest, RemoveUserFeeRequest,
    SetAssetDefaultFeeRequest, SetUserFeeRequest, UserAssetFeeEntry,
};

use crate::{
    ApiError,
    http_utils::request_response::empty_json_reply,
    server::{
        Server,
        db::models::{NewAssetDefaultFee, NewUserFee},
    },
};

// -------------
// | Constants |
// -------------

/// The minimum fee value
const MIN_FEE: f32 = 0.0; // 0%
/// The maximum fee value
const MAX_FEE: f32 = 0.01; // 1%

/// The error message to return if a fee is invalid
const INVALID_FEE_MSG: &str = "Fee must be between 0.0 and 0.01";
/// The error message to return if a ticker is invalid
const INVALID_TICKER_MSG: &str = "Ticker must be 3 uppercase letters";

// -----------
// | Helpers |
// -----------

/// Validate a fee value
pub fn validate_fee(fee: f32) -> Result<(), ApiError> {
    let valid_fee = (MIN_FEE..=MAX_FEE).contains(&fee);
    valid_fee.then_some(()).ok_or(ApiError::bad_request(INVALID_FEE_MSG))
}

/// Validate an asset's ticker
pub fn validate_ticker(ticker: &str) -> Result<(), ApiError> {
    Token::maybe_from_ticker(ticker).ok_or(ApiError::bad_request(INVALID_TICKER_MSG)).map(|_| ())
}

// --------------------
// | Endpoint Methods |
// --------------------

impl Server {
    // --- Getters --- //

    /// Get the per-asset, per-user fee for all users and assets
    #[instrument(skip_all)]
    pub async fn get_all_user_fees(
        &self,
        path: FullPath,
        headers: HeaderMap,
    ) -> Result<Json, Rejection> {
        self.authorize_management_request(&path, &headers, &Bytes::new() /* body */)?;

        // Get the cartesian product with fee inheritance in a single query
        let user_asset_fees = self.get_user_asset_fees_with_defaults().await?;
        let default_fees = self.get_all_asset_default_fees_query().await?;

        let response = GetAllFeesResponse {
            user_asset_fees: user_asset_fees.into_iter().map(UserAssetFeeEntry::from).collect(),
            default_fees: default_fees.into_iter().map(AssetDefaultFeeEntry::from).collect(),
        };
        Ok(warp::reply::json(&response))
    }

    // --- Setters --- //

    /// Set the default fee for a given asset
    #[instrument(skip_all)]
    pub async fn set_asset_default_fee(
        &self,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Json, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        // Parse the request body and validate it
        let req: SetAssetDefaultFeeRequest =
            serde_json::from_slice(&body).map_err(ApiError::bad_request)?;
        validate_fee(req.fee)?;
        validate_ticker(&req.asset)?;

        // Create the new default fee entry and upsert it in the database
        let new_default_fee = NewAssetDefaultFee::new(req.asset, req.fee);
        self.set_asset_default_fee_query(new_default_fee).await?;

        Ok(empty_json_reply())
    }

    /// Remove the default fee for a given asset
    #[instrument(skip_all)]
    pub async fn remove_asset_default_fee(
        &self,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Json, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        // Parse the request body and validate it
        let req: RemoveAssetDefaultFeeRequest =
            serde_json::from_slice(&body).map_err(ApiError::bad_request)?;
        validate_ticker(&req.asset)?;

        self.remove_asset_default_fee_query(req.asset).await?;
        Ok(empty_json_reply())
    }

    /// Set the per-user fee override for a given asset
    #[instrument(skip_all)]
    pub async fn set_user_fee_override(
        &self,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Json, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        // Parse the request body and validate it
        let req: SetUserFeeRequest =
            serde_json::from_slice(&body).map_err(ApiError::bad_request)?;
        validate_fee(req.fee)?;
        validate_ticker(&req.asset)?;

        // Create the new user fee entry, upsert it in the database
        let new_user_fee = NewUserFee::new(req.user_id, req.asset, req.fee);
        self.set_user_fee_query(new_user_fee).await?;

        Ok(empty_json_reply())
    }

    /// Remove the per-user fee override for a given asset
    #[instrument(skip_all)]
    pub async fn remove_user_fee_override(
        &self,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Json, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        // Parse the request body and remove the user fee override
        let req: RemoveUserFeeRequest =
            serde_json::from_slice(&body).map_err(ApiError::bad_request)?;
        validate_ticker(&req.asset)?;

        self.remove_user_fee_query(req.user_id, req.asset).await?;
        Ok(empty_json_reply())
    }
}
