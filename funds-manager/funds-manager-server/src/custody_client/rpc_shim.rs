//! Implements an opinionated handler for Ethereum JSON-RPC requests, which
//! can be used, for example, by EIP-1193 compliant Javascript clients.

use alloy_json_rpc::{
    ErrorPayload, Request, Response as JsonRpcResponse, ResponsePayload, RpcError, RpcResult,
};
use fireblocks_sdk::{
    apis::transactions_api::CreateTransactionParams,
    models::{
        unsigned_message::Type as MessageType, CreateTransactionResponse, ExtraParameters,
        ExtraParametersRawMessageData, SourceTransferPeerPath, TransactionOperation,
        TransactionRequest, TransactionStatus, UnsignedMessage,
    },
};
use renegade_arbitrum_client::constants::Chain;
use serde_json::Value;
use tracing::error;

// Note: We deliberately use the Ethers implementation of EIP-712 TypedData
// rather than Alloy's, because Alloy will fail to deserialize TypedData which
// contains type identifiers that are not valid Solidity types.
// For example, a Hyperliquid withdrawal action contains a field with the type
// identifier `HyperliquidTransaction:Withdraw`. Due to the usage of the `:`
// character, Alloy will fail to deserialize the TypedData.
use ethers::types::{
    transaction::eip712::{EIP712Domain, TypedData},
    U256,
};

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
/// The EIP-712 domain name for Hyperliquid L1 actions
const HYPERLIQUID_L1_ACTION_DOMAIN: &str = "Exchange";
/// The EIP-712 domain name for Hyperliquid user actions
const HYPERLIQUID_USER_ACTION_DOMAIN: &str = "HyperliquidSignTransaction";
/// The set of allowed EIP-712 domains for typed data signing requests.
const ALLOWED_EIP712_DOMAIN_NAMES: [&str; 2] =
    [HYPERLIQUID_L1_ACTION_DOMAIN, HYPERLIQUID_USER_ACTION_DOMAIN];

/// The error message emitted when an unsupported RPC method is requested.
const ERR_UNSUPPORTED_METHOD: &str = "Unsupported RPC method";
/// The error message emitted when an unsupported chain is configured.
const ERR_UNSUPPORTED_CHAIN: &str = "Unsupported chain";
/// The error message emitted when the Hyperliquid vault is not found.
const ERR_HYPERLIQUID_VAULT_NOT_FOUND: &str = "Hyperliquid vault not found";
/// The error message emitted when no addresses are found for the Hyperliquid
/// vault.
const ERR_NO_ADDRESSES: &str = "No addresses found for Hyperliquid vault";
/// The error message emitted when the signing account is invalid.
const ERR_INVALID_SIGNING_ACCOUNT: &str = "Invalid signing account";
/// The error message emitted when the chain ID in an EIP-712 domain is invalid.
const ERR_INVALID_CHAIN_ID: &str = "Invalid chain ID";
/// The error message emitted when the EIP-712 domain name is invalid.
const ERR_INVALID_DOMAIN_NAME: &str = "Invalid domain name";
/// The error message emitted when a signature is not found in the Fireblocks
/// transaction response.
const ERR_SIGNATURE_NOT_FOUND: &str = "Signature not found in Fireblocks transaction response";

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

// ----------
// | Macros |
// ----------

/// A macro for creating an invalid parameters JSON-RPC error response.
macro_rules! invalid_params {
    () => {
        RpcError::err_resp(ErrorPayload::invalid_params())
    };
}

impl CustodyClient {
    // ------------
    // | Handlers |
    // ------------

    /// Handle an incoming JSON-RPC request, wrapping the result in a
    /// `JsonRpcResponse` appropriately.
    pub async fn handle_rpc_request(
        &self,
        request: JsonRpcRequest,
    ) -> JsonRpcResponse<Value, Value> {
        let id = request.meta.id.clone();
        let result = self.try_handle_rpc_request(request).await;

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
        request: JsonRpcRequest,
    ) -> FundsManagerRpcResult<Value> {
        let method: &str = &request.meta.method;

        match method {
            ETH_SIGN_TYPED_DATA_V4_METHOD => {
                self.handle_eth_sign_typed_data_v4_request(request).await.map(Value::from)
            },
            ETH_ACCOUNTS_METHOD => self.handle_eth_accounts_request().await.map(Value::from),
            _ => Err(RpcError::UnsupportedFeature(ERR_UNSUPPORTED_METHOD)),
        }
    }

    /// Get the list of accounts managed by the custody client.
    /// Currently, we only support RPC requests pertaining to the Hyperliquid
    /// keypair.
    async fn handle_eth_accounts_request(&self) -> FundsManagerRpcResult<Vec<String>> {
        let hyperliquid_vault_id = self.get_hyperliquid_vault_id().await?;
        let hyperliquid_address = self.get_hyperliquid_address(&hyperliquid_vault_id).await?;
        Ok(vec![hyperliquid_address])
    }

    /// Handle an incoming `eth_signTypedData_v4` JSON-RPC request
    async fn handle_eth_sign_typed_data_v4_request(
        &self,
        request: JsonRpcRequest,
    ) -> FundsManagerRpcResult<String> {
        // Parse request parameters
        let (address, typed_data) = parse_sign_typed_data_params(request.params)?;

        let hyperliquid_vault_id = self.get_hyperliquid_vault_id().await?;

        // Validate request parameters
        self.validate_signing_account(&address, &hyperliquid_vault_id).await?;
        self.validate_typed_data(&typed_data)?;

        // Sign the typed data
        let note = self.generate_typed_data_note(HYPERLIQUID_VAULT_NAME, &typed_data);
        let tx_resp = self
            .send_fireblocks_typed_data_signature_request(hyperliquid_vault_id, typed_data, note)
            .await?;

        let tx = self.poll_fireblocks_transaction(&tx_resp.id).await?;
        if tx.status != TransactionStatus::Completed {
            let err_msg = format!("Typed data signature request unsuccessful: {}", tx.status);
            error!("{err_msg}");
            return Err(FundsManagerError::fireblocks(err_msg).into());
        }

        let signature = tx
            .signed_messages
            .and_then(|signed_messages| signed_messages.first().cloned())
            .and_then(|signed_message| signed_message.signature)
            .and_then(|signature| {
                signature.r.zip(signature.s).zip(signature.v).map(|((r, s), v)| {
                    let v_hex = hex::encode([v as u8]);
                    format!("0x{r}{s}{v_hex}")
                })
            })
            .ok_or(FundsManagerError::fireblocks(ERR_SIGNATURE_NOT_FOUND))?;

        Ok(signature)
    }

    // -----------
    // | Helpers |
    // -----------

    /// Validate the signing account of an `eth_signTypedData_v4` JSON-RPC
    /// request.
    async fn validate_signing_account(
        &self,
        address: &str,
        hyperliquid_vault_id: &str,
    ) -> FundsManagerRpcResult<()> {
        if address != self.get_hyperliquid_address(hyperliquid_vault_id).await?.as_str() {
            return Err(FundsManagerError::json_rpc(ERR_INVALID_SIGNING_ACCOUNT).into());
        }
        Ok(())
    }

    /// Validate the contents of the typed data requested to be signed.
    fn validate_typed_data(&self, typed_data: &TypedData) -> FundsManagerRpcResult<()> {
        self.validate_domain(&typed_data.domain)?;
        // TODO: More validation
        Ok(())
    }

    /// Validate the EIP-712 signing domain of a typed data request.
    ///
    /// Currently, we only support Hyperliquid typed data signing requests.
    fn validate_domain(&self, domain: &EIP712Domain) -> Result<(), FundsManagerError> {
        match domain.chain_id {
            None => return Err(FundsManagerError::json_rpc(ERR_INVALID_CHAIN_ID)),
            Some(chain_id) => {
                if chain_id != U256::from(self.chain_id) {
                    return Err(FundsManagerError::json_rpc(ERR_INVALID_CHAIN_ID));
                }
            },
        }

        if domain.name.is_none() {
            return Err(FundsManagerError::json_rpc(ERR_INVALID_DOMAIN_NAME));
        }

        match domain.name.as_ref() {
            None => Err(FundsManagerError::json_rpc(ERR_INVALID_DOMAIN_NAME)),
            Some(name) if ALLOWED_EIP712_DOMAIN_NAMES.contains(&name.as_str()) => Ok(()),
            _ => Err(FundsManagerError::json_rpc(ERR_INVALID_DOMAIN_NAME)),
        }
    }

    /// Get the Fireblocks asset ID for the native asset (ETH) of the configured
    /// chain.
    fn get_native_eth_asset_id(&self) -> FundsManagerRpcResult<String> {
        match self.chain {
            Chain::Mainnet => Ok(ARB_MAINNET_ETH_ASSET_ID.to_string()),
            Chain::Testnet => Ok(ARB_TESTNET_ETH_ASSET_ID.to_string()),
            _ => Err(RpcError::UnsupportedFeature(ERR_UNSUPPORTED_CHAIN)),
        }
    }

    /// Get the Fireblocks vault ID for the Hyperliquid vault.
    async fn get_hyperliquid_vault_id(&self) -> FundsManagerRpcResult<String> {
        let hyperliquid_vault = self
            .get_vault_account(HYPERLIQUID_VAULT_NAME)
            .await?
            .ok_or(FundsManagerError::fireblocks(ERR_HYPERLIQUID_VAULT_NOT_FOUND))?;

        Ok(hyperliquid_vault.id)
    }

    /// Get the address of the Hyperliquid account.
    /// This is expected to be the only address managing native ETH in the
    /// Hyperliquid vault.
    async fn get_hyperliquid_address(
        &self,
        hyperliquid_vault_id: &str,
    ) -> FundsManagerRpcResult<String> {
        let asset_id = self.get_native_eth_asset_id()?;
        let addresses = self
            .fireblocks_client
            .addresses(hyperliquid_vault_id, &asset_id)
            .await
            .map_err(FundsManagerError::from)?;

        let addr = addresses.first().ok_or(FundsManagerError::fireblocks(ERR_NO_ADDRESSES))?;
        Ok(addr.address.clone())
    }

    /// Generate a note for a Fireblocks transaction that signs a typed data
    /// message
    fn generate_typed_data_note(&self, vault_name: &str, typed_data: &TypedData) -> String {
        let action = &typed_data.primary_type;
        format!("Signing {action} using {vault_name}")
    }

    /// Send a request to Fireblocks to sign a typed data message.
    async fn send_fireblocks_typed_data_signature_request(
        &self,
        vault_id: String,
        typed_data: TypedData,
        note: String,
    ) -> FundsManagerRpcResult<CreateTransactionResponse> {
        let source = SourceTransferPeerPath { id: Some(vault_id), ..Default::default() };
        let content = serde_json::to_value(&typed_data).map_err(FundsManagerError::json_rpc)?;
        let extra_parameters = ExtraParameters {
            raw_message_data: Some(ExtraParametersRawMessageData {
                messages: Some(vec![UnsignedMessage {
                    r#type: Some(MessageType::Eip712),
                    content,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let asset_id = self.get_native_eth_asset_id()?;

        let params = CreateTransactionParams::builder()
            .transaction_request(TransactionRequest {
                operation: Some(TransactionOperation::TypedMessage),
                source: Some(source),
                extra_parameters: Some(extra_parameters),
                note: Some(note),
                asset_id: Some(asset_id),
                ..Default::default()
            })
            .build();

        let resp = self
            .fireblocks_client
            .transactions_api()
            .create_transaction(params)
            .await
            .map_err(FundsManagerError::fireblocks)?;

        Ok(resp)
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Parse the parameters of an `eth_signTypedData_v4` JSON-RPC request,
/// namely the address of the signing account and the typed data to be signed.
fn parse_sign_typed_data_params(mut params: Value) -> FundsManagerRpcResult<(String, TypedData)> {
    let mut params_iter = params.as_array_mut().ok_or(invalid_params!())?.iter_mut();

    let address =
        params_iter.next().and_then(|value| value.as_str()).ok_or(invalid_params!())?.to_string();

    let raw_data = params_iter.next().ok_or(invalid_params!())?.take();
    let raw_data_str = serde_json::to_string(&raw_data).expect("Failed to re-serialize typed data");

    let typed_data: TypedData = serde_json::from_value(raw_data).map_err(|err| {
        error!("Failed to deserialize typed data: {}", err);
        RpcError::deser_err(err, raw_data_str)
    })?;

    Ok((address, typed_data))
}
