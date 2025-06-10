//! API types for managing and redeeming fees

use renegade_api::types::ApiWallet;
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

// -------------
// | Api Types |
// -------------

/// The response containing fee wallets
#[derive(Debug, Serialize, Deserialize)]
pub struct FeeWalletsResponse {
    /// The wallets managed by the funds manager
    pub wallets: Vec<ApiWallet>,
}

/// The request body for withdrawing a fee balance
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WithdrawFeeBalanceRequest {
    /// The ID of the wallet to withdraw from
    pub wallet_id: Uuid,
    /// The mint of the asset to withdraw
    pub mint: String,
}
