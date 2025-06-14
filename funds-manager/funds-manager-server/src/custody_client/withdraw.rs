//! Withdrawal methods for custodied funds
use std::str::FromStr;

use crate::{
    error::FundsManagerError,
    helpers::{get_secret, round_up},
};
use alloy::signers::local::PrivateKeySigner;
use fireblocks_sdk::{
    apis::{transactions_api::CreateTransactionParams, Api},
    models::{
        DestinationTransferPeerPath, SourceTransferPeerPath, TransactionOperation,
        TransactionRequest, TransactionRequestAmount, TransactionStatus, TransferPeerPathType,
    },
};
use renegade_common::types::{
    chain::Chain,
    token::{Token, USDC_TICKER},
};
use tracing::info;

use super::{CustodyClient, DepositWithdrawSource};

// -------------
// | Constants |
// -------------

/// The address of the Hyperliquid bridge on Arbitrum mainnet
const MAINNET_HYPERLIQUID_BRIDGE_ADDRESS: &str = "0x2df1c51e09aecf9cacb7bc98cb1742757f163df7";
/// The address of the Hyperliquid bridge on Arbitrum testnet
const TESTNET_HYPERLIQUID_BRIDGE_ADDRESS: &str = "0x08cfc1B6b2dCF36A1480b99353A354AA8AC56f89";
/// The address of the dummy USDC token used by Hyperliquid's testnet deployment
const TESTNET_HYPERLIQUID_USDC_ADDRESS: &str = "0x1baAbB04529D43a73232B713C0FE471f7c7334d5";
/// The number of decimals for USDC
const USDC_DECIMALS: i64 = 6;

/// The error message for when the Hyperliquid bridge is not found in the
/// set of Fireblocks whitelisted contracts
const ERR_HYPERLIQUID_BRIDGE_NOT_FOUND: &str = "Hyperliquid bridge not found";
/// The error message for when the USDC asset is not found in Fireblocks
const ERR_USDC_ASSET_NOT_FOUND: &str = "USDC asset not found";
/// The error message for when the chain is not supported
const ERR_UNSUPPORTED_CHAIN: &str = "Unsupported chain";

// ---------------
// | Client impl |
// ---------------

impl CustodyClient {
    // ------------
    // | Handlers |
    // ------------

    /// Withdraw from hot wallet custody with a provided token address
    pub(crate) async fn withdraw_from_hot_wallet(
        &self,
        source: DepositWithdrawSource,
        destination_address: &str,
        token_address: &str,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        // Find the wallet for the given destination and check its balance
        let wallet = self.get_hot_wallet_by_vault(&source.vault_name(self.chain)).await?;
        let bal = self.get_erc20_balance(token_address, &wallet.address).await?;
        if bal < amount {
            return Err(FundsManagerError::Custom("Insufficient balance".to_string()));
        }

        // Fetch the wallet private key
        let wallet = self.get_hot_wallet_private_key(&wallet.address).await?;

        // Execute the erc20 transfer
        let tx = self.erc20_transfer(token_address, destination_address, amount, wallet).await?;
        info!(
            "Withdrew {amount} {token_address} from hot wallet to {destination_address}. Tx: {:?}",
            tx.transaction_hash
        );

        Ok(())
    }

    /// Withdraw funds from custody
    pub(crate) async fn withdraw_from_fireblocks(
        &self,
        source: DepositWithdrawSource,
        mint: &str,
        withdraw_amount: f64,
    ) -> Result<(), FundsManagerError> {
        let vault_name = source.vault_name(self.chain);
        let hot_wallet = self.get_hot_wallet_by_vault(&vault_name).await?;

        // Get the vault account and asset to transfer from
        let vault = self
            .get_vault_account(&vault_name)
            .await?
            .ok_or_else(|| FundsManagerError::Custom("Vault not found".to_string()))?;
        let asset_id = self.get_asset_id_for_address(mint).await?.ok_or_else(|| {
            FundsManagerError::Custom(format!("Asset not found for mint: {mint}"))
        })?;

        // Check if the available balance is sufficient
        let available_str = vault
            .assets
            .iter()
            .find(|a| a.id == asset_id)
            .map(|acct| acct.available.clone())
            .unwrap_or_default();

        let available = available_str.parse::<f64>().map_err(FundsManagerError::parse)?;

        if available < withdraw_amount {
            return Err(FundsManagerError::Custom(format!(
                "Insufficient balance. Available: {}, Requested: {}",
                available, withdraw_amount
            )));
        }

        // Transfer
        let wallet_id = hot_wallet.internal_wallet_id.to_string();
        self.transfer_from_vault(
            vault.id,
            asset_id,
            wallet_id,
            TransferPeerPathType::InternalWallet,
            withdraw_amount,
        )
        .await
    }

    /// Withdraw gas
    pub(crate) async fn withdraw_gas(
        &self,
        amount: f64,
        to: &str,
    ) -> Result<(), FundsManagerError> {
        // Check the gas wallet's balance
        let gas_vault_name = DepositWithdrawSource::Gas.vault_name(self.chain);
        let gas_wallet = self.get_hot_wallet_by_vault(&gas_vault_name).await?;
        let bal = self.get_ether_balance(&gas_wallet.address).await?;
        if bal < amount {
            return Err(FundsManagerError::custom("Insufficient balance"));
        }

        // Fetch the gas wallet's private key
        let secret_name = Self::hot_wallet_secret_name(&gas_wallet.address);
        let private_key = get_secret(&secret_name, &self.aws_config).await?;
        let wallet =
            PrivateKeySigner::from_str(private_key.as_str()).map_err(FundsManagerError::parse)?;

        // Execute the transfer
        let tx = self.transfer_ether(to, amount, wallet).await?;
        info!("Withdrew {amount} ETH from gas wallet to {to}. Tx: {:#}", tx.transaction_hash);

        Ok(())
    }

    /// Withdraw USDC to Hyperliquid from the quoter hot wallet
    pub(crate) async fn withdraw_to_hyperliquid(
        &self,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        // Round up to the nearest USDC_DECIMALS decimal place
        let rounded_amount = round_up(amount, USDC_DECIMALS)?;

        let hyperliquid_vault_id = self.get_hyperliquid_vault_id().await?;
        let hyperliquid_address = self.get_hyperliquid_address().await?;

        let hot_wallet = self.get_quoter_hot_wallet().await?;

        let usdc_mint = self.get_hyperliquid_usdc_mint()?;

        let hl_bal = self.get_erc20_balance(&usdc_mint, &hyperliquid_address).await?;
        if hl_bal < amount {
            // We round up the amount to transfer to account for
            // potential floating point precision issues.
            let amount_to_transfer = round_up(rounded_amount - hl_bal, USDC_DECIMALS)?;
            let bal = self.get_erc20_balance(&usdc_mint, &hot_wallet.address).await?;
            if bal < amount_to_transfer {
                return Err(FundsManagerError::Custom("Insufficient balance".to_string()));
            }

            // Transfer the USDC to the Hyperliquid account
            self.transfer_to_hyperliquid_account(
                amount_to_transfer,
                &hot_wallet.address,
                &hyperliquid_address,
                &usdc_mint,
            )
            .await?;
        }

        // Transfer the USDC from the Hyperliquid account to the bridge.
        // This is necessary so that the USDC is credited to the same account on the
        // Hyperliquid L1.
        self.bridge_to_hyperliquid(rounded_amount, &usdc_mint, hyperliquid_vault_id).await
    }

    // -----------
    // | Helpers |
    // -----------

    /// Transfer USDC from the quoter hot wallet to the Hyperliquid account's
    /// keypair
    async fn transfer_to_hyperliquid_account(
        &self,
        amount: f64,
        quoter_hot_wallet_addr: &str,
        hyperliquid_addr: &str,
        usdc_mint: &str,
    ) -> Result<(), FundsManagerError> {
        // Fetch the quoter hot wallet private key
        let hot_wallet = self.get_hot_wallet_private_key(quoter_hot_wallet_addr).await?;

        // Transfer the USDC to the address used by the Hyperliquid account.
        let tx = self.erc20_transfer(usdc_mint, hyperliquid_addr, amount, hot_wallet).await?;
        let tx_hash = tx.transaction_hash;

        self.poll_fireblocks_external_transaction(tx_hash).await?;

        info!("Withdrew {amount} USDC from hot wallet to {hyperliquid_addr}. Tx: {:#x}", tx_hash);

        Ok(())
    }

    /// Bridge USDC to Hyperliquid using the given account
    async fn bridge_to_hyperliquid(
        &self,
        amount: f64,
        usdc_mint: &str,
        hyperliquid_vault_id: String,
    ) -> Result<(), FundsManagerError> {
        let hyperliquid_bridge_id = self.get_hyperliquid_bridge_id().await?;

        let asset_id = self
            .get_asset_id_for_address(usdc_mint)
            .await?
            .ok_or(FundsManagerError::fireblocks(ERR_USDC_ASSET_NOT_FOUND))?;

        self.transfer_from_vault(
            hyperliquid_vault_id,
            asset_id,
            hyperliquid_bridge_id,
            TransferPeerPathType::ExternalWallet,
            amount,
        )
        .await
    }

    /// Get the Fireblocks ID of the whitelisted Hyperliquid bridge
    /// contract.
    ///
    /// We store the bridge address as an "external wallet" in Fireblocks
    /// to allow ERC20 transfers to it.
    async fn get_hyperliquid_bridge_id(&self) -> Result<String, FundsManagerError> {
        let bridge_address = match self.chain {
            Chain::ArbitrumOne => MAINNET_HYPERLIQUID_BRIDGE_ADDRESS,
            Chain::ArbitrumSepolia => TESTNET_HYPERLIQUID_BRIDGE_ADDRESS,
            _ => return Err(FundsManagerError::custom(ERR_UNSUPPORTED_CHAIN)),
        };

        let whitelisted_wallets = self
            .fireblocks_client
            .sdk
            .apis()
            .whitelisted_external_wallets_api()
            .get_external_wallets()
            .await?;

        for wallet in whitelisted_wallets {
            let wallet_id = wallet.id;
            for asset in wallet.assets {
                if let Some(address) = asset.address {
                    if address.to_lowercase() == bridge_address.to_lowercase() {
                        return Ok(wallet_id);
                    }
                }
            }
        }

        Err(FundsManagerError::fireblocks(ERR_HYPERLIQUID_BRIDGE_NOT_FOUND))
    }

    /// Transfer an asset from a vault to the given destination
    async fn transfer_from_vault(
        &self,
        vault_id: String,
        asset_id: String,
        dest_id: String,
        dest_type: TransferPeerPathType,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        let note =
            format!("Transfer {amount} {asset_id} from vault {vault_id} to destination {dest_id}");

        let source = SourceTransferPeerPath { id: Some(vault_id), ..Default::default() };

        let destination = DestinationTransferPeerPath {
            r#type: dest_type,
            id: Some(dest_id),
            ..Default::default()
        };

        let amount = TransactionRequestAmount::Number(amount);

        let params = CreateTransactionParams::builder()
            .transaction_request(TransactionRequest {
                operation: Some(TransactionOperation::Transfer),
                source: Some(source),
                destination: Some(destination),
                asset_id: Some(asset_id),
                amount: Some(amount),
                note: Some(note),
                ..Default::default()
            })
            .build();

        let resp = self.fireblocks_client.sdk.transactions_api().create_transaction(params).await?;

        let tx = self.poll_fireblocks_transaction(&resp.id).await?;
        if tx.status != TransactionStatus::Completed && tx.status != TransactionStatus::Confirming {
            let err_msg = format!("Transaction failed: {}", tx.status);
            return Err(FundsManagerError::Fireblocks(err_msg));
        }

        Ok(())
    }

    /// Get the USDC mint for the Hyperliquid account
    pub(crate) fn get_hyperliquid_usdc_mint(&self) -> Result<String, FundsManagerError> {
        match self.chain {
            Chain::ArbitrumOne => Ok(Token::from_ticker_on_chain(USDC_TICKER, self.chain).addr),
            Chain::ArbitrumSepolia => Ok(TESTNET_HYPERLIQUID_USDC_ADDRESS.to_string()),
            _ => Err(FundsManagerError::custom(ERR_UNSUPPORTED_CHAIN)),
        }
    }
}
