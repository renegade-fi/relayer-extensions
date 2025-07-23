//! Fee management API endpoints

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --------------------------
// | Request/Response Types |
// --------------------------

/// A request to set the default fee for an asset
#[derive(Debug, Serialize, Deserialize)]
pub struct SetAssetDefaultFeeRequest {
    /// The asset identifier
    pub asset: String,
    /// The fee rate as a floating point value
    pub fee: f32,
}

/// A request to set a user-specific fee override
#[derive(Debug, Serialize, Deserialize)]
pub struct SetUserFeeRequest {
    /// The user's API key ID
    pub user_id: Uuid,
    /// The asset identifier
    pub asset: String,
    /// The fee rate as a floating point value
    pub fee: f32,
}

/// Response containing all fee configurations
#[derive(Debug, Serialize, Deserialize)]
pub struct GetAllFeesResponse {
    /// All user-asset fee pairs (cartesian product with defaults applied)
    pub user_asset_fees: Vec<UserAssetFeeEntry>,
    /// All asset default fees for reference
    pub default_fees: Vec<AssetDefaultFeeEntry>,
}

// -------------
// | API Types |
// -------------

/// A user-specific fee override entry
#[derive(Debug, Serialize, Deserialize)]
pub struct UserFeeEntry {
    /// The user's API key ID
    pub id: Uuid,
    /// The asset ticker
    pub asset: String,
    /// The fee rate as a floating point value
    pub fee: f32,
}

/// A default fee entry for an asset
#[derive(Debug, Serialize, Deserialize)]
pub struct AssetDefaultFeeEntry {
    /// The asset ticker
    pub asset: String,
    /// The fee rate as a floating point value
    pub fee: f32,
}

/// A fee entry for a specific user-asset pair
#[derive(Debug, Serialize, Deserialize)]
pub struct UserAssetFeeEntry {
    /// The user's API key ID
    pub user_id: Uuid,
    /// The user's API key description
    pub user_description: String,
    /// The asset ticker
    pub asset: String,
    /// The fee rate as a floating point value
    pub fee: f32,
    /// Whether this fee is a user-specific override (true) or inherited from
    /// default (false)
    pub is_override: bool,
}
