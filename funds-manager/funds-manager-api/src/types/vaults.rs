//! API types for vault management

use serde::{Deserialize, Serialize};

use super::hot_wallets::TokenBalance;

// --------------
// | Api Routes |
// --------------

/// The route to get the balances of a vault
pub const GET_VAULT_BALANCES_ROUTE: &str = "get-vault-balances";

// -------------
// | Api Types |
// -------------

/// The request to get the balances of a vault
#[derive(Debug, Serialize, Deserialize)]
pub struct GetVaultBalancesRequest {
    /// The name of the vault
    pub vault: String,
}

/// The response containing the balances of a vault
#[derive(Debug, Serialize, Deserialize)]
pub struct VaultBalancesResponse {
    /// The balances of the vault
    pub balances: Vec<TokenBalance>,
}
