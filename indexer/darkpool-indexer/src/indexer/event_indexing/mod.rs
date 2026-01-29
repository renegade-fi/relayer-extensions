//! Indexer-specific logic for indexing onchain events

use alloy::{
    primitives::{B256, TxHash},
    sol_types::SolCall,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_darkpool_types::balance::DarkpoolBalanceShare;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    cancelPrivateOrderCall, cancelPublicOrderCall, depositCall, depositNewBalanceCall,
    payPrivateProtocolFeeCall, payPrivateRelayerFeeCall, payPublicProtocolFeeCall,
    payPublicRelayerFeeCall, settleExternalMatchCall, settleMatchCall, withdrawCall,
};

use crate::{
    darkpool_client::utils::get_selector,
    indexer::{
        Indexer, error::IndexerError, event_indexing::party_settlement_data::PartySettlementData,
    },
    state_transitions::{
        StateTransition,
        cancel_order::CancelOrderTransition,
        create_balance::{BalanceCreationData, CreateBalanceTransition},
        deposit::DepositTransition,
        pay_protocol_fee::PayProtocolFeeTransition,
        pay_relayer_fee::PayRelayerFeeTransition,
        withdraw::WithdrawTransition,
    },
};

pub mod party_settlement_data;

// --------------------------
// | Indexer Implementation |
// --------------------------

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
                self.compute_deposit_transition(nullifier, tx_hash, &calldata).await
            },
            withdrawCall::SELECTOR => {
                self.compute_withdraw_transition(nullifier, tx_hash, &calldata).await
            },
            payPublicProtocolFeeCall::SELECTOR => {
                self.compute_pay_public_protocol_fee_transition(nullifier, tx_hash, &calldata).await
            },
            payPrivateProtocolFeeCall::SELECTOR => {
                self.compute_pay_private_protocol_fee_transition(nullifier, tx_hash, &calldata)
                    .await
            },
            payPublicRelayerFeeCall::SELECTOR => {
                self.compute_pay_public_relayer_fee_transition(nullifier, tx_hash, &calldata).await
            },
            payPrivateRelayerFeeCall::SELECTOR => {
                self.compute_pay_private_relayer_fee_transition(nullifier, tx_hash, &calldata).await
            },
            settleMatchCall::SELECTOR => {
                let (party0_settlement_data, party1_settlement_data) =
                    PartySettlementData::pair_from_settle_match_calldata(&calldata)?;

                let maybe_party0_state_transition = party0_settlement_data
                    .get_state_transition_for_nullifier(&self.darkpool_client, nullifier, tx_hash)
                    .await?;

                let maybe_party1_state_transition = party1_settlement_data
                    .get_state_transition_for_nullifier(&self.darkpool_client, nullifier, tx_hash)
                    .await?;

                maybe_party0_state_transition.or(maybe_party1_state_transition).ok_or(
                    IndexerError::invalid_party_settlement_data(format!(
                        "nullifier {nullifier} not spent by either party in match tx {tx_hash:#x}"
                    )),
                )
            },
            cancelPrivateOrderCall::SELECTOR => {
                self.compute_cancel_order_transition(nullifier, tx_hash).await
            },
            _ => Err(IndexerError::invalid_selector(selector)),
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

        match selector {
            depositNewBalanceCall::SELECTOR => self
                .compute_create_balance_transition_from_deposit(recovery_id, tx_hash, &calldata)
                .await
                .map(Some),
            settleMatchCall::SELECTOR => {
                let (party0_settlement_data, party1_settlement_data) =
                    PartySettlementData::pair_from_settle_match_calldata(&calldata)?;

                let maybe_party0_state_transition = party0_settlement_data
                    .get_state_transition_for_recovery_id(
                        &self.darkpool_client,
                        recovery_id,
                        tx_hash,
                    )
                    .await?;

                let maybe_party1_state_transition = party1_settlement_data
                    .get_state_transition_for_recovery_id(
                        &self.darkpool_client,
                        recovery_id,
                        tx_hash,
                    )
                    .await?;

                Ok(maybe_party0_state_transition.or(maybe_party1_state_transition))
            },
            _ => Ok(None),
        }
    }

    /// Get the state transition associated with the creation of a new public
    /// intent
    pub async fn get_state_transition_for_public_intent_creation(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
    ) -> Result<StateTransition, IndexerError> {
        let creation_call =
            self.darkpool_client.find_public_intent_creation_call(intent_hash, tx_hash).await?;

        let calldata = creation_call.input;
        let selector = get_selector(&calldata);

        match selector {
            settleMatchCall::SELECTOR => {
                let (party0_settlement_data, party1_settlement_data) =
                    PartySettlementData::pair_from_settle_match_calldata(&calldata)?;

                let maybe_party0_state_transition = party0_settlement_data
                    .get_state_transition_for_public_intent_creation(
                        &self.darkpool_client,
                        intent_hash,
                        tx_hash,
                    )
                    .await?;

                let maybe_party1_state_transition = party1_settlement_data
                    .get_state_transition_for_public_intent_creation(
                        &self.darkpool_client,
                        intent_hash,
                        tx_hash,
                    )
                    .await?;

                maybe_party0_state_transition.or(maybe_party1_state_transition).ok_or(
                    IndexerError::invalid_party_settlement_data(
                        "no public intent creation found in settle match call",
                    ),
                )
            },
            settleExternalMatchCall::SELECTOR => {
                let call =
                    settleExternalMatchCall::abi_decode(&calldata).map_err(IndexerError::parse)?;

                let settlement_data = PartySettlementData::from_settle_external_match_call(&call)?;

                settlement_data
                    .get_state_transition_for_public_intent_creation(
                        &self.darkpool_client,
                        intent_hash,
                        tx_hash,
                    )
                    .await?
                    .ok_or(IndexerError::invalid_party_settlement_data(
                        "no public intent creation found in settle external match call",
                    ))
            },
            _ => Err(IndexerError::invalid_selector(selector)),
        }
    }

    /// Get the state transition associated with the update of a public intent
    pub async fn get_state_transition_for_public_intent_update(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
    ) -> Result<StateTransition, IndexerError> {
        let update_call =
            self.darkpool_client.find_public_intent_update_call(intent_hash, tx_hash).await?;

        let calldata = update_call.input;
        let selector = get_selector(&calldata);

        match selector {
            settleMatchCall::SELECTOR => {
                let (party0_settlement_data, party1_settlement_data) =
                    PartySettlementData::pair_from_settle_match_calldata(&calldata)?;

                let maybe_party0_state_transition = party0_settlement_data
                    .get_state_transition_for_public_intent_update(
                        &self.darkpool_client,
                        intent_hash,
                        tx_hash,
                    )
                    .await?;

                let maybe_party1_state_transition = party1_settlement_data
                    .get_state_transition_for_public_intent_update(
                        &self.darkpool_client,
                        intent_hash,
                        tx_hash,
                    )
                    .await?;

                maybe_party0_state_transition.or(maybe_party1_state_transition).ok_or(
                    IndexerError::invalid_party_settlement_data(
                        "no public intent update found in settle match call",
                    ),
                )
            },
            settleExternalMatchCall::SELECTOR => {
                let call =
                    settleExternalMatchCall::abi_decode(&calldata).map_err(IndexerError::parse)?;

                let settlement_data = PartySettlementData::from_settle_external_match_call(&call)?;

                settlement_data
                    .get_state_transition_for_public_intent_update(
                        &self.darkpool_client,
                        intent_hash,
                        tx_hash,
                    )
                    .await?
                    .ok_or(IndexerError::invalid_party_settlement_data(
                        "no public intent update found in settle external match call",
                    ))
            },
            _ => Err(IndexerError::invalid_selector(selector)),
        }
    }

    /// Get the state transition associated with the cancellation of a public
    /// intent
    pub async fn get_state_transition_for_public_intent_cancellation(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
    ) -> Result<StateTransition, IndexerError> {
        let cancellation_call =
            self.darkpool_client.find_public_intent_cancellation_call(intent_hash, tx_hash).await?;

        let calldata = cancellation_call.input;
        let selector = get_selector(&calldata);

        match selector {
            cancelPublicOrderCall::SELECTOR => {
                // TODO: Implement public intent cancellation state transition
                unimplemented!("Public intent cancellation state transition not yet implemented")
            },
            _ => Err(IndexerError::invalid_selector(selector)),
        }
    }
}

// -------------------
// | Private Helpers |
// -------------------

impl Indexer {
    /// Compute a `CreateBalance` state transition associated with the
    /// newly-registered recovery ID in a `depositNewBalance` call
    async fn compute_create_balance_transition_from_deposit(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let deposit_new_balance_call =
            depositNewBalanceCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let public_share: DarkpoolBalanceShare =
            deposit_new_balance_call.newBalanceProofBundle.statement.newBalanceShares.into();

        let balance_creation_data = BalanceCreationData::DepositNewBalance { public_share };

        Ok(StateTransition::CreateBalance(CreateBalanceTransition {
            recovery_id,
            block_number,
            balance_creation_data,
        }))
    }

    /// Compute a `Deposit` state transition associated with the now-spent
    /// nullifier in a `deposit` call
    async fn compute_deposit_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let deposit_call = depositCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_amount_public_share =
            u256_to_scalar(deposit_call.depositProofBundle.statement.newAmountShare);

        Ok(StateTransition::Deposit(DepositTransition {
            nullifier,
            block_number,
            new_amount_public_share,
        }))
    }

    /// Compute a `Withdraw` state transition associated with the now-spent
    /// nullifier in a `withdraw` call
    async fn compute_withdraw_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let withdraw_call = withdrawCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_amount_public_share =
            u256_to_scalar(withdraw_call.withdrawalProofBundle.statement.newAmountShare);

        Ok(StateTransition::Withdraw(WithdrawTransition {
            nullifier,
            block_number,
            new_amount_public_share,
        }))
    }

    /// Compute a `PayProtocolFee` state transition associated with the
    /// now-spent nullifier in a `payPublicProtocolFee` call
    async fn compute_pay_public_protocol_fee_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let pay_public_protocol_fee_call =
            payPublicProtocolFeeCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_protocol_fee_public_share = u256_to_scalar(
            pay_public_protocol_fee_call.proofBundle.statement.newProtocolFeeBalanceShare,
        );

        Ok(StateTransition::PayProtocolFee(PayProtocolFeeTransition {
            nullifier,
            block_number,
            new_protocol_fee_public_share,
        }))
    }

    /// Compute a `PayProtocolFee` state transition associated with the
    /// now-spent nullifier in a `payPrivateProtocolFee` call
    async fn compute_pay_private_protocol_fee_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let pay_private_protocol_fee_call =
            payPrivateProtocolFeeCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_protocol_fee_public_share = u256_to_scalar(
            pay_private_protocol_fee_call.proofBundle.statement.newProtocolFeeBalanceShare,
        );

        Ok(StateTransition::PayProtocolFee(PayProtocolFeeTransition {
            nullifier,
            block_number,
            new_protocol_fee_public_share,
        }))
    }

    /// Compute a `PayRelayerFee` state transition associated with the
    /// now-spent nullifier in a `payPublicRelayerFee` call
    async fn compute_pay_public_relayer_fee_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let pay_public_relayer_fee_call =
            payPublicRelayerFeeCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_relayer_fee_public_share = u256_to_scalar(
            pay_public_relayer_fee_call.proofBundle.statement.newRelayerFeeBalanceShare,
        );

        Ok(StateTransition::PayRelayerFee(PayRelayerFeeTransition {
            nullifier,
            block_number,
            new_relayer_fee_public_share,
        }))
    }

    /// Compute a `PayRelayerFee` state transition associated with the
    /// now-spent nullifier in a `payPrivateRelayerFee` call
    async fn compute_pay_private_relayer_fee_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let pay_private_relayer_fee_call =
            payPrivateRelayerFeeCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let new_relayer_fee_public_share = u256_to_scalar(
            pay_private_relayer_fee_call.proofBundle.statement.newRelayerFeeBalanceShare,
        );

        Ok(StateTransition::PayRelayerFee(PayRelayerFeeTransition {
            nullifier,
            block_number,
            new_relayer_fee_public_share,
        }))
    }

    /// Compute a `CancelOrder` state transition associated with the now-spent
    /// nullifier in a `cancelOrder` call
    async fn compute_cancel_order_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        Ok(StateTransition::CancelOrder(CancelOrderTransition { nullifier, block_number }))
    }
}
