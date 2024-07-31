//! The API for the funds manager
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use renegade_api::types::ApiWallet;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --------------
// | Api Routes |
// --------------

/// The ping route
pub const PING_ROUTE: &str = "ping";
/// The route through which a client may start the fee indexing process
pub const INDEX_FEES_ROUTE: &str = "index-fees";
/// The route through which a client may start the fee redemption process
pub const REDEEM_FEES_ROUTE: &str = "redeem-fees";

/// The route to retrieve the address to deposit custody funds to
pub const GET_DEPOSIT_ADDRESS_ROUTE: &str = "deposit-address";
/// The route to withdraw funds from custody
pub const WITHDRAW_CUSTODY_ROUTE: &str = "withdraw";

/// The route to withdraw gas from custody
pub const WITHDRAW_GAS_ROUTE: &str = "withdraw-gas";

/// The route to get fee wallets
pub const GET_FEE_WALLETS_ROUTE: &str = "get-fee-wallets";
/// The route to withdraw a fee balance
pub const WITHDRAW_FEE_BALANCE_ROUTE: &str = "withdraw-fee-balance";

/// The route to transfer funds from a hot wallet to its backing vault
pub const TRANSFER_TO_VAULT_ROUTE: &str = "transfer-to-vault";
/// The route to withdraw funds from a hot wallet to Fireblocks
pub const WITHDRAW_TO_HOT_WALLET_ROUTE: &str = "withdraw-to-hot-wallet";

// -------------
// | Api Types |
// -------------

// --- Fee Indexing & Management --- //

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

// --- Quoters --- //

/// A response containing the deposit address
#[derive(Debug, Serialize, Deserialize)]
pub struct DepositAddressResponse {
    /// The deposit address
    pub address: String,
}

/// The request body for withdrawing funds from custody
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WithdrawFundsRequest {
    /// The mint of the asset to withdraw
    pub mint: String,
    /// The amount of funds to withdraw
    pub amount: f64,
    /// The address to withdraw to
    pub address: String,
}

// --- Gas --- //

// Update request body name and documentation
/// The request body for withdrawing gas from custody
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WithdrawGasRequest {
    /// The amount of gas to withdraw
    pub amount: f64,
    /// The address to withdraw to
    pub destination_address: String,
}

// --- Hot Wallets --- //

/// The request body for creating a hot wallet
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateHotWalletRequest {
    /// The name of the vault backing the hot wallet
    pub vault: String,
}

/// The response containing the hot wallet's address
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateHotWalletResponse {
    /// The address of the hot wallet
    pub address: String,
}

/// The response containing hot wallet balances
#[derive(Debug, Serialize, Deserialize)]
pub struct HotWalletBalancesResponse {
    /// The list of hot wallets with their balances
    pub wallets: Vec<WalletWithBalances>,
}

/// A hot wallet with its balances
#[derive(Debug, Serialize, Deserialize)]
pub struct WalletWithBalances {
    /// The address of the hot wallet
    pub address: String,
    /// The balances of various tokens
    pub balances: Vec<TokenBalance>,
}

/// A balance for a specific token
#[derive(Debug, Serialize, Deserialize)]
pub struct TokenBalance {
    /// The mint address of the token
    pub mint: String,
    /// The balance amount
    pub amount: u128,
}

/// The request body for transferring funds from a hot wallet to its backing
/// vault
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferToVaultRequest {
    /// The address of the hot wallet
    pub hot_wallet_address: String,
    /// The mint of the asset to transfer
    pub mint: String,
    /// The amount to transfer
    pub amount: f64,
}

/// The request body for transferring from Fireblocks to a hot wallet
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WithdrawToHotWalletRequest {
    /// The name of the vault to withdraw from
    pub vault: String,
    /// The mint of the asset to transfer
    pub mint: String,
    /// The amount to transfer
    pub amount: f64,
}
