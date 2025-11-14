//! Indexer-specific logic for indexing onchain events

use alloy::{hex, primitives::TxHash, sol_types::SolCall};
use renegade_circuit_types::{balance::BalanceShare, traits::BaseType};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    depositCall, depositNewBalanceCall, payFeesCall, settleMatchCall, withdrawCall,
};

use crate::{
    darkpool_client::utils::get_selector,
    indexer::{Indexer, error::IndexerError},
    state_transitions::{
        StateTransition, create_balance::CreateBalanceTransition, deposit::DepositTransition,
        pay_fees::PayFeesTransition, withdraw::WithdrawTransition,
    },
};

impl Indexer {
    /// Get the state transition associated with the spending of the given
    /// nullifier in the given transaction
    pub async fn get_state_transition_for_nullifier(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
    ) -> Result<StateTransition, IndexerError> {
        let nullifying_call = self.darkpool_client.find_nullifying_call(nullifier, tx_hash).await?;

        let calldata = nullifying_call.input;
        let selector = get_selector(&calldata);

        match selector {
            depositCall::SELECTOR => {
                self.compute_deposit_state_transition(nullifier, tx_hash, &calldata).await
            },
            withdrawCall::SELECTOR => {
                self.compute_withdraw_state_transition(nullifier, tx_hash, &calldata).await
            },
            payFeesCall::SELECTOR => {
                self.compute_pay_fees_state_transition(nullifier, tx_hash, &calldata).await
            },
            settleMatchCall::SELECTOR => todo!(),
            _ => Err(IndexerError::InvalidSelector(hex::encode_prefixed(selector))),
        }
    }

    /// Get the state transition associated with the registration of the given
    /// recovery ID in the given transaction, if any
    pub async fn get_state_transition_for_recovery_id(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        let recovery_id_registration_call =
            self.darkpool_client.find_recovery_id_registration_call(recovery_id, tx_hash).await?;

        let calldata = recovery_id_registration_call.input;
        let selector = get_selector(&calldata);

        let state_transition = match selector {
            depositNewBalanceCall::SELECTOR => {
                self.compute_create_balance_state_transition(recovery_id, tx_hash, &calldata)
                    .await?
            },
            // TODO: Implement getting intent registration transitions
            _ => return Ok(None),
        };

        Ok(Some(state_transition))
    }

    /// Compute a `CreateBalance` state transition associated with the
    /// newly-registered recovery ID in a `depositNewBalance` call
    async fn compute_create_balance_state_transition(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let deposit_new_balance_call =
            depositNewBalanceCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let mut public_shares_iter = deposit_new_balance_call
            .newBalanceProofBundle
            .statement
            .newBalancePublicShares
            .iter()
            .map(u256_to_scalar);

        let public_share = BalanceShare::from_scalars(&mut public_shares_iter);

        Ok(StateTransition::CreateBalance(CreateBalanceTransition {
            recovery_id,
            block_number,
            public_share,
        }))
    }

    /// Compute a `Deposit` state transition associated with the now-spent
    /// nullifier in a `deposit` call
    async fn compute_deposit_state_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let deposit_call = depositCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_amount_public_share =
            u256_to_scalar(&deposit_call.depositProofBundle.statement.newAmountShare);

        Ok(StateTransition::Deposit(DepositTransition {
            nullifier,
            block_number,
            new_amount_public_share,
        }))
    }

    /// Compute a `Withdraw` state transition associated with the now-spent
    /// nullifier in a `withdraw` call
    async fn compute_withdraw_state_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let withdraw_call = withdrawCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_amount_public_share =
            u256_to_scalar(&withdraw_call.withdrawalProofBundle.statement.newAmountShare);

        Ok(StateTransition::Withdraw(WithdrawTransition {
            nullifier,
            block_number,
            new_amount_public_share,
        }))
    }

    /// Compute a `PayFees` state transition associated with the now-spent
    /// nullifier in a `payFees` call
    async fn compute_pay_fees_state_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let pay_fees_call = payFeesCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let [
            new_relayer_fee_public_share_u256,
            new_protocol_fee_public_share_u256,
            new_amount_public_share_u256,
        ] = pay_fees_call.feePaymentProofBundle.statement.newBalancePublicShares;

        let new_relayer_fee_public_share = u256_to_scalar(&new_relayer_fee_public_share_u256);
        let new_protocol_fee_public_share = u256_to_scalar(&new_protocol_fee_public_share_u256);
        let new_amount_public_share = u256_to_scalar(&new_amount_public_share_u256);

        Ok(StateTransition::PayFees(PayFeesTransition {
            nullifier,
            block_number,
            new_relayer_fee_public_share,
            new_protocol_fee_public_share,
            new_amount_public_share,
        }))
    }
}
