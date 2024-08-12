//! API types for managing hot wallets

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --------------
// | Api Routes |
// --------------

/// The route to transfer funds from a hot wallet to its backing vault
pub const TRANSFER_TO_VAULT_ROUTE: &str = "transfer-to-vault";
/// The route to withdraw funds from a hot wallet to Fireblocks
pub const WITHDRAW_TO_HOT_WALLET_ROUTE: &str = "withdraw-to-hot-wallet";

// -------------
// | Api Types |
// -------------

/// The request body for creating a hot wallet
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateHotWalletRequest {
    /// The name of the vault backing the hot wallet
    pub vault: String,
    /// The internal wallet ID to associate with the hot wallet
    pub internal_wallet_id: Uuid,
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
