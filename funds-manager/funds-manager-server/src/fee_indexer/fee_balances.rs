//! Fetch the balances of redeemed fees

use crate::custody_client::DepositWithdrawSource;
use crate::db::models::RenegadeWalletMetadata;
use crate::error::FundsManagerError;
use ethers::{
    core::k256::ecdsa::SigningKey,
    types::{Signature, U256},
    utils::keccak256,
};
use num_bigint::BigUint;
use renegade_api::{
    http::wallet::{WalletUpdateAuthorization, WithdrawBalanceRequest},
    types::ApiWallet,
};
use renegade_arbitrum_client::{
    conversion::to_contract_external_transfer, helpers::serialize_calldata,
};
use renegade_circuit_types::{
    keychain::SecretSigningKey,
    transfers::{ExternalTransfer, ExternalTransferDirection},
    Amount,
};
use renegade_common::types::wallet::{derivation::derive_wallet_keychain, Wallet};
use renegade_util::hex::biguint_from_hex_string;
use uuid::Uuid;

use super::Indexer;

impl Indexer {
    // -------------
    // | Interface |
    // -------------

    /// Fetch fee balances for wallets managed by the funds manager
    pub async fn fetch_fee_wallets(&self) -> Result<Vec<ApiWallet>, FundsManagerError> {
        // Query the wallets and fetch from the relayer
        let wallet_metadata = self.get_all_wallets().await?;
        let mut wallets = Vec::with_capacity(wallet_metadata.len());
        for meta in wallet_metadata.into_iter() {
            let wallet = self.fetch_wallet(meta).await?;
            wallets.push(wallet);
        }

        Ok(wallets)
    }

    /// Withdraw a fee balance for a specific wallet and mint
    pub async fn withdraw_fee_balance(
        &self,
        wallet_id: Uuid,
        mint: String,
    ) -> Result<(), FundsManagerError> {
        // Fetch the Renegade wallet
        let wallet_metadata = self.get_wallet_by_id(&wallet_id).await?;
        let api_wallet = self.fetch_wallet(wallet_metadata.clone()).await?;
        let old_wallet = Wallet::try_from(api_wallet).map_err(FundsManagerError::custom)?;
        let wallet_key = old_wallet.key_chain.symmetric_key();

        // Get the deposit address for the fee withdrawal
        let deposit_address =
            self.custody_client.get_deposit_address(DepositWithdrawSource::FeeRedemption).await?;

        // Send a withdrawal request to the relayer
        let req = Self::build_withdrawal_request(&mint, &deposit_address, &old_wallet)?;
        self.relayer_client.withdraw_balance(wallet_metadata.id, mint, req, &wallet_key).await?;

        Ok(())
    }

    // -----------
    // | Helpers |
    // -----------

    /// Fetch a wallet given its metadata
    ///
    /// This is done by:
    ///     1. Fetch the wallet's key from secrets manager
    ///     2. Use the key to fetch the wallet from the relayer
    async fn fetch_wallet(
        &self,
        wallet_metadata: RenegadeWalletMetadata,
    ) -> Result<ApiWallet, FundsManagerError> {
        // Get the wallet's private key from secrets manager
        let eth_key = self.get_wallet_private_key(&wallet_metadata).await?;

        // Derive the wallet keychain
        let wallet_keychain =
            derive_wallet_keychain(&eth_key, self.chain_id).map_err(FundsManagerError::custom)?;
        let wallet_key = wallet_keychain.symmetric_key();

        // Fetch the wallet from the relayer and replace the keychain so that we have
        // access to the full set of secret keys
        let mut wallet =
            self.relayer_client.get_wallet(wallet_metadata.id, &wallet_key).await?.wallet;
        wallet.key_chain = wallet_keychain.into();

        Ok(wallet)
    }

    /// Build a withdrawal request
    fn build_withdrawal_request(
        mint: &str,
        to: &str,
        old_wallet: &Wallet,
    ) -> Result<WithdrawBalanceRequest, FundsManagerError> {
        // Withdraw the balance from the wallet
        let mut new_wallet = old_wallet.clone();
        let mint_bigint = biguint_from_hex_string(mint).map_err(FundsManagerError::custom)?;
        let bal = new_wallet.get_balance(&mint_bigint).cloned().ok_or_else(|| {
            FundsManagerError::custom(format!("No balance found for mint {mint}"))
        })?;

        if bal.amount == 0 {
            return Err(FundsManagerError::custom(format!("Balance for mint {mint} is 0")));
        }
        new_wallet.withdraw(&mint_bigint, bal.amount).map_err(FundsManagerError::custom)?;
        new_wallet.reblind_wallet();

        // Sign the commitment to the new wallet and the transfer to the deposit address
        let root_key =
            old_wallet.key_chain.secret_keys.sk_root.as_ref().expect("root key not present");
        let commitment_sig = old_wallet
            .sign_commitment(new_wallet.get_wallet_share_commitment())
            .expect("failed to sign wallet commitment");

        let dest_bigint = biguint_from_hex_string(to).map_err(FundsManagerError::custom)?;
        let transfer_sig = Self::authorize_withdrawal(
            root_key,
            mint_bigint.clone(),
            bal.amount,
            dest_bigint.clone(),
        )?;

        let update_auth = WalletUpdateAuthorization {
            statement_sig: commitment_sig.to_vec(),
            new_root_key: None,
        };

        Ok(WithdrawBalanceRequest {
            destination_addr: dest_bigint,
            amount: BigUint::from(bal.amount),
            update_auth,
            external_transfer_sig: transfer_sig.to_vec(),
        })
    }

    /// Authorize a withdrawal from the darkpool
    fn authorize_withdrawal(
        root_key: &SecretSigningKey,
        mint: BigUint,
        amount: Amount,
        to: BigUint,
    ) -> Result<Signature, FundsManagerError> {
        let converted_key: SigningKey = root_key.try_into().expect("key conversion failed");

        // Construct a transfer
        let transfer = ExternalTransfer {
            mint,
            amount,
            direction: ExternalTransferDirection::Withdrawal,
            account_addr: to,
        };

        // Sign the transfer with the root key
        let contract_transfer =
            to_contract_external_transfer(&transfer).map_err(FundsManagerError::custom)?;
        let buf = serialize_calldata(&contract_transfer).map_err(FundsManagerError::custom)?;
        let digest = keccak256(&buf);
        let (sig, recovery_id) =
            converted_key.sign_prehash_recoverable(&digest).map_err(FundsManagerError::custom)?;

        Ok(Signature {
            r: U256::from_big_endian(&sig.r().to_bytes()),
            s: U256::from_big_endian(&sig.s().to_bytes()),
            v: recovery_id.to_byte() as u64,
        })
    }
}
