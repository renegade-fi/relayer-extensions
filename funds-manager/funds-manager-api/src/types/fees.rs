//! API types for managing and redeeming fees

use renegade_api::types::ApiAccount;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --------------
// | Api Routes |
// --------------

/// The route through which a client may start the fee indexing process
pub const INDEX_FEES_ROUTE: &str = "index-fees";
/// The route through which a client may start the fee redemption process
pub const REDEEM_FEES_ROUTE: &str = "redeem-fees";
/// The route to get fee wallets
pub const GET_FEE_WALLETS_ROUTE: &str = "get-fee-wallets";
/// The route to withdraw a fee balance
pub const WITHDRAW_FEE_BALANCE_ROUTE: &str = "withdraw-fee-balance";
/// The route to get the hot wallet address for fee redemption
pub const GET_FEE_HOT_WALLET_ADDRESS_ROUTE: &str = "get-hot-wallet-address";
/// The route to get the unredeemed fee totals
pub const GET_UNREDEEMED_FEE_TOTALS_ROUTE: &str = "get-unredeemed-fee-totals";

// -------------
// | Api Types |
// -------------

/// The response containing fee accounts
#[derive(Debug, Serialize, Deserialize)]
pub struct FeeWalletsResponse {
    /// The accounts managed by the funds manager
    pub wallets: Vec<ApiAccount>,
}

/// The request body for withdrawing a fee balance
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WithdrawFeeBalanceRequest {
    /// The ID of the wallet to withdraw from
    pub wallet_id: Uuid,
    /// The mint of the asset to withdraw
    pub mint: String,
}

/// A single unredeemed fee total
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnredeemedFeeTotal {
    /// The mint of the fee asset
    pub mint: String,
    /// The nominal amount of the fee
    pub amount: u128,
}

/// The response containing the unredeemed fee totals
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnredeemedFeeTotalsResponse {
    /// The unredeemed fee totals
    pub totals: Vec<UnredeemedFeeTotal>,
}
