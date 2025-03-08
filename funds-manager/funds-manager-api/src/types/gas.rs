//! API types for gas funding and gas wallet tracking
use serde::{Deserialize, Serialize};

// --------------
// | Api Routes |
// --------------

/// The route to withdraw gas from custody
pub const WITHDRAW_GAS_ROUTE: &str = "withdraw-gas";
/// The route to refill gas for all active wallets
pub const REFILL_GAS_ROUTE: &str = "refill-gas";
/// The route to register a gas wallet for a peer
pub const REGISTER_GAS_WALLET_ROUTE: &str = "register-gas-wallet";
/// The route to report active peers
pub const REPORT_ACTIVE_PEERS_ROUTE: &str = "report-active-peers";
/// The route to refill the gas sponsor contract
pub const REFILL_GAS_SPONSOR_ROUTE: &str = "refill-gas-sponsor";

// -------------
// | Api Types |
// -------------

/// The request body for withdrawing gas from custody
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WithdrawGasRequest {
    /// The amount of gas to withdraw
    pub amount: f64,
    /// The address to withdraw to
    pub destination_address: String,
}

/// The request body for refilling gas for all active wallets
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefillGasRequest {
    /// The amount of gas to top up each wallet to
    pub amount: f64,
}

/// The response containing the gas wallet's address
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateGasWalletResponse {
    /// The address of the gas wallet
    pub address: String,
}

/// A request to allocate a gas wallet for a peer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterGasWalletRequest {
    /// The peer ID of the peer to allocate a gas wallet for
    pub peer_id: String,
}

/// The response containing an newly active gas wallet's key
///
/// Clients will hit the corresponding endpoint to register a gas wallet with
/// the funds manager when they spin up
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterGasWalletResponse {
    /// The key of the active gas wallet
    pub key: String,
}

/// A request reporting active peers in the network
///
/// The funds manager uses such a request to mark gas wallets as active or
/// inactive
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportActivePeersRequest {
    /// The list of active peers
    pub peers: Vec<String>,
}
