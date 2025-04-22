//! Implements an opinionated handler for Ethereum JSON-RPC requests, which
//! can be used, for example, by EIP-1193 compliant Javascript clients.

use alloy_json_rpc::{Request, Response as JsonRpcResponse, ResponsePayload, RpcError, RpcResult};
use renegade_arbitrum_client::constants::Chain;
use serde_json::Value;

use crate::error::FundsManagerError;

use super::CustodyClient;

// -------------
// | Constants |
// -------------

/// The method name for the `eth_accounts` JSON-RPC method.
const ETH_ACCOUNTS_METHOD: &str = "eth_accounts";
/// The method name for the `eth_signTypedData_v4` JSON-RPC method.
const ETH_SIGN_TYPED_DATA_V4_METHOD: &str = "eth_signTypedData_v4";

/// The Fireblocks asset ID for ETH on Arbitrum mainnet
const ARB_MAINNET_ETH_ASSET_ID: &str = "ETH-AETH";
/// The Fireblocks asset ID for ETH on Arbitrum testnet
const ARB_TESTNET_ETH_ASSET_ID: &str = "ETH-AETH_SEPOLIA";
/// The name of the Fireblocks vault custodying the Hyperliquid keypair
const HYPERLIQUID_VAULT_NAME: &str = "Hyperliquid";

/// The error message emitted when an unsupported RPC method is requested.
const ERR_UNSUPPORTED_METHOD: &str = "Unsupported RPC method";
/// The error message emitted when an unsupported chain is configured.
const ERR_UNSUPPORTED_CHAIN: &str = "Unsupported chain";
/// The error message emitted when the Hyperliquid vault is not found.
const ERR_HYPERLIQUID_VAULT_NOT_FOUND: &str = "Hyperliquid vault not found";
/// The error message emitted when no addresses are found for the Hyperliquid
/// vault.
const ERR_NO_ADDRESSES: &str = "No addresses found for Hyperliquid vault";

// ---------
// | Types |
// ---------

/// A type alias for the JSON-RPC request type.
pub type JsonRpcRequest = Request<Value>;

/// A type alias for the result type of the funds manager's JSON-RPC handler.
/// This is generic over the success type, expects `FundsManagerError` as the
/// custom transport error type, and expects a JSON `Value` as the RPC error
/// response type.
type FundsManagerRpcResult<T> = RpcResult<T, FundsManagerError, Value>;

impl CustodyClient {
    /// Handle an incoming JSON-RPC request, wrapping the result in a
    /// `JsonRpcResponse` appropriately.
    pub async fn handle_rpc_request(
        &self,
        request: JsonRpcRequest,
    ) -> JsonRpcResponse<Value, Value> {
        let id = request.meta.id.clone();
        let result = self.try_handle_rpc_request(&request).await;

        match result {
            Ok(result) => JsonRpcResponse { id, payload: ResponsePayload::Success(result) },
            Err(error) => match error.as_error_resp() {
                Some(error_payload) => {
                    JsonRpcResponse { id, payload: ResponsePayload::Failure(error_payload.clone()) }
                },
                None => JsonRpcResponse::internal_error_message(id, error.to_string().into()),
            },
        }
    }

    /// Handle an incoming JSON-RPC request,
    /// validating the request and returning an arbitrary result value.
    async fn try_handle_rpc_request(
        &self,
        request: &JsonRpcRequest,
    ) -> FundsManagerRpcResult<Value> {
        let method: &str = &request.meta.method;

        match method {
            ETH_SIGN_TYPED_DATA_V4_METHOD => todo!(),
            ETH_ACCOUNTS_METHOD => self.handle_eth_accounts_request().await.map(Value::from),
            _ => Err(RpcError::UnsupportedFeature(ERR_UNSUPPORTED_METHOD)),
        }
    }

    /// Get the list of accounts managed by the custody client.
    /// Currently, we only support RPC requests pertaining to the Hyperliquid
    /// keypair.
    async fn handle_eth_accounts_request(&self) -> FundsManagerRpcResult<Vec<String>> {
        let asset_id = match self.chain {
            Chain::Mainnet => ARB_MAINNET_ETH_ASSET_ID,
            Chain::Testnet => ARB_TESTNET_ETH_ASSET_ID,
            _ => return Err(RpcError::UnsupportedFeature(ERR_UNSUPPORTED_CHAIN)),
        };
        let hyperliquid_vault = self
            .get_vault_account(HYPERLIQUID_VAULT_NAME)
            .await?
            .ok_or(FundsManagerError::fireblocks(ERR_HYPERLIQUID_VAULT_NOT_FOUND))?;

        let client = self.get_fireblocks_client()?;
        let (addresses, _rid) = client
            .addresses(hyperliquid_vault.id, &asset_id)
            .await
            .map_err(FundsManagerError::from)?;

        let addr = addresses.first().ok_or(FundsManagerError::fireblocks(ERR_NO_ADDRESSES))?;

        Ok(vec![addr.address.clone()])
    }
}
