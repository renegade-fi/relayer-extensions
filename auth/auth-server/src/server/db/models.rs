//! DB model types for the auth server
#![allow(missing_docs, clippy::missing_docs_in_private_items)]
#![allow(trivial_bounds)]

use std::time::SystemTime;

use auth_server_api::{
    fee_management::{AssetDefaultFeeEntry, UserAssetFeeEntry},
    key_management::ApiKey as UserFacingApiKey,
};
use diesel::prelude::*;
use uuid::Uuid;

use crate::server::db::schema::{api_keys, asset_default_fees, rate_limits, user_fees};

#[derive(Queryable, Selectable, Clone)]
#[diesel(table_name = api_keys)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiKey {
    pub id: Uuid,
    pub encrypted_key: String,
    pub description: String,
    #[allow(dead_code)]
    pub created_at: SystemTime,
    pub is_active: bool,
    pub rate_limit_whitelisted: bool,
}

impl From<ApiKey> for UserFacingApiKey {
    fn from(key: ApiKey) -> Self {
        let created_at = key.created_at.duration_since(SystemTime::UNIX_EPOCH).unwrap();
        Self {
            id: key.id,
            description: key.description,
            is_active: key.is_active,
            rate_limit_whitelisted: key.rate_limit_whitelisted,
            created_at: created_at.as_secs(),
        }
    }
}

#[derive(Insertable)]
#[diesel(table_name = api_keys)]
pub struct NewApiKey {
    pub id: Uuid,
    pub encrypted_key: String,
    pub description: String,
}

impl NewApiKey {
    /// Create a new API key
    pub fn new(id: Uuid, encrypted_key: String, description: String) -> Self {
        Self { id, encrypted_key, description }
    }
}

impl From<NewApiKey> for ApiKey {
    fn from(key: NewApiKey) -> Self {
        Self {
            id: key.id,
            encrypted_key: key.encrypted_key,
            description: key.description,
            created_at: SystemTime::now(),
            is_active: true,
            rate_limit_whitelisted: false,
        }
    }
}

#[derive(Insertable)]
#[diesel(table_name = user_fees)]
pub struct NewUserFee {
    pub id: Uuid,
    pub asset: String,
    pub fee: f32,
}

impl NewUserFee {
    /// Create a new user fee entry
    pub fn new(id: Uuid, asset: String, fee: f32) -> Self {
        Self { id, asset, fee }
    }
}

#[derive(Queryable, Selectable, Clone)]
#[diesel(table_name = asset_default_fees)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AssetDefaultFee {
    pub asset: String,
    pub fee: f32,
}

// Conversion functions between DB models and API types
impl From<AssetDefaultFee> for AssetDefaultFeeEntry {
    fn from(db_fee: AssetDefaultFee) -> Self {
        Self { asset: db_fee.asset, fee: db_fee.fee }
    }
}

#[derive(Insertable)]
#[diesel(table_name = asset_default_fees)]
pub struct NewAssetDefaultFee {
    pub asset: String,
    pub fee: f32,
}

impl NewAssetDefaultFee {
    /// Create a new asset default fee entry
    pub fn new(asset: String, fee: f32) -> Self {
        Self { asset, fee }
    }
}

/// Result of the joined query for user-asset fees with defaults
#[derive(QueryableByName, Clone)]
pub struct UserAssetFeeQueryResult {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub user_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub user_description: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub asset: String,
    #[diesel(sql_type = diesel::sql_types::Float)]
    pub fee: f32,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub is_override: bool,
}

impl From<UserAssetFeeQueryResult> for UserAssetFeeEntry {
    fn from(query_result: UserAssetFeeQueryResult) -> Self {
        Self {
            user_id: query_result.user_id,
            user_description: query_result.user_description,
            asset: query_result.asset,
            fee: query_result.fee,
            is_override: query_result.is_override,
        }
    }
}

/// Result of a fee query with fallback logic
#[derive(diesel::QueryableByName)]
pub struct FeeResult {
    #[diesel(sql_type = diesel::sql_types::Float)]
    pub fee: f32,
}

/// Result of a rate limit query
#[derive(diesel::QueryableByName)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RateLimitResult {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub requests_per_minute: i32,
}

/// Rate limit method enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateLimitMethod {
    Quote,
    Assemble,
}

impl RateLimitMethod {
    /// Convert to database string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            RateLimitMethod::Quote => "quote",
            RateLimitMethod::Assemble => "assemble",
        }
    }

    /// Parse from database string representation
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "quote" => Some(RateLimitMethod::Quote),
            "assemble" => Some(RateLimitMethod::Assemble),
            _ => None,
        }
    }
}

#[derive(Queryable, Selectable, Clone)]
#[diesel(table_name = rate_limits)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RateLimit {
    pub api_key_id: Uuid,
    pub method: String,
    pub requests_per_minute: i32,
}

/// New rate limit for insertion (not using Insertable due to custom enum type)
pub struct NewRateLimit {
    /// The API key ID
    pub api_key_id: Uuid,
    /// The rate limit method
    pub method: RateLimitMethod,
    /// The requests per minute limit
    pub requests_per_minute: u32,
}

impl NewRateLimit {
    /// Create a new rate limit entry
    pub fn new(api_key_id: Uuid, method: RateLimitMethod, requests_per_minute: u32) -> Self {
        Self { api_key_id, method, requests_per_minute }
    }
}
