//! Indexer-specific logic for indexing onchain events

use alloy::{
    primitives::{B256, TxHash},
    sol_types::SolCall,
};
use renegade_circuit_types::{balance::BalanceShare, traits::BaseType};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    cancelOrderCall, depositCall, depositNewBalanceCall, payPrivateProtocolFeeCall,
    payPrivateRelayerFeeCall, payPublicProtocolFeeCall, payPublicRelayerFeeCall, settleMatchCall,
    withdrawCall,
};

use crate::{
    darkpool_client::utils::get_selector,
    indexer::{
        Indexer,
        error::IndexerError,
        event_indexing::{
            types::settlement_bundle::SettlementBundleData,
            utils::{
                try_decode_balance_settlement_data, try_decode_intent_creation_data,
                try_decode_intent_settlement_data,
            },
        },
    },
    state_transitions::{
        StateTransition, cancel_order::CancelOrderTransition,
        create_balance::CreateBalanceTransition, create_intent::CreateIntentTransition,
        create_public_intent::CreatePublicIntentTransition, deposit::DepositTransition,
        pay_protocol_fee::PayProtocolFeeTransition, pay_relayer_fee::PayRelayerFeeTransition,
        settle_match_into_balance::SettleMatchIntoBalanceTransition,
        settle_match_into_intent::SettleMatchIntoIntentTransition,
        settle_match_into_public_intent::SettleMatchIntoPublicIntentTransition,
        withdraw::WithdrawTransition,
    },
};

pub mod types;
mod utils;

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
                let maybe_settle_match_into_balance_transition = self
                    .try_compute_settle_match_into_balance_transition(nullifier, tx_hash, &calldata)
                    .await?;

                let maybe_settle_match_into_intent_transition = self
                    .try_compute_settle_match_into_intent_transition(nullifier, tx_hash, &calldata)
                    .await?;

                maybe_settle_match_into_balance_transition
                    .or(maybe_settle_match_into_intent_transition)
                    .ok_or(IndexerError::invalid_settlement_bundle(
                        "no balance or intent nullified in match tx 0x{tx_hash:#x}",
                    ))
            },
            cancelOrderCall::SELECTOR => {
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

        let maybe_state_transition = match selector {
            depositNewBalanceCall::SELECTOR => {
                Some(self.compute_create_balance_transition(recovery_id, tx_hash, &calldata).await?)
            },
            settleMatchCall::SELECTOR => {
                self.try_compute_create_intent_transition(recovery_id, tx_hash, &calldata).await?
            },
            _ => None,
        };

        Ok(maybe_state_transition)
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

        if selector != settleMatchCall::SELECTOR {
            return Err(IndexerError::invalid_selector(selector));
        }

        self.compute_create_public_intent_transition(intent_hash, tx_hash, &calldata).await
    }

    /// Get the state transition associated with the update of a public intent
    pub async fn get_state_transition_for_public_intent_update(
        &self,
        intent_hash: B256,
        version: u64,
        tx_hash: TxHash,
    ) -> Result<StateTransition, IndexerError> {
        let update_call =
            self.darkpool_client.find_public_intent_update_call(intent_hash, tx_hash).await?;

        let calldata = update_call.input;
        let selector = get_selector(&calldata);

        match selector {
            settleMatchCall::SELECTOR => {
                self.compute_settle_match_into_public_intent_transition(
                    intent_hash,
                    version,
                    tx_hash,
                    &calldata,
                )
                .await
            },
            // TODO: Handle intent cancellation once ABI is finalized
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
    async fn compute_create_balance_transition(
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
    async fn compute_deposit_transition(
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
    async fn compute_withdraw_transition(
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
            &pay_public_protocol_fee_call.proofBundle.statement.newProtocolFeeBalanceShare,
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
            &pay_private_protocol_fee_call.proofBundle.statement.newProtocolFeeBalanceShare,
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
            &pay_public_relayer_fee_call.proofBundle.statement.newRelayerFeeBalanceShare,
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
            &pay_private_relayer_fee_call.proofBundle.statement.newRelayerFeeBalanceShare,
        );

        Ok(StateTransition::PayRelayerFee(PayRelayerFeeTransition {
            nullifier,
            block_number,
            new_relayer_fee_public_share,
        }))
    }

    /// Try to compute a `SettleMatchIntoBalance` state transition associated
    /// with the now-spent nullifier in a `settleMatch` call.
    ///
    /// Returns `None` if the spent nullifier does not match the input/output
    /// balance nullifier of either party.
    async fn try_compute_settle_match_into_balance_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<Option<StateTransition>, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let settle_match_call =
            settleMatchCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let maybe_party0_balance_settlement_data = try_decode_balance_settlement_data(
            nullifier,
            &settle_match_call.party0SettlementBundle,
            &settle_match_call.obligationBundle,
            true, // is_party0
        )?;

        let maybe_party1_balance_settlement_data = try_decode_balance_settlement_data(
            nullifier,
            &settle_match_call.party1SettlementBundle,
            &settle_match_call.obligationBundle,
            false, // is_party0
        )?;

        let maybe_balance_settlement_data =
            maybe_party0_balance_settlement_data.or(maybe_party1_balance_settlement_data);

        // If we could not decode balance settlement data for either party,
        // the spent nullifier must pertain to one of the intents nullified in the
        // match.
        if maybe_balance_settlement_data.is_none() {
            return Ok(None);
        }

        let balance_settlement_data = maybe_balance_settlement_data.unwrap();

        Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
            nullifier,
            block_number,
            balance_settlement_data,
        })))
    }

    /// Try to compute a `CreateIntent` state transition associated with the
    /// newly-registered recovery ID in a `settleMatch` call.
    ///
    /// Returns `None` if the registered recovery ID does not match the recovery
    /// ID of any newly-created intents in the match.
    async fn try_compute_create_intent_transition(
        &self,
        recovery_id: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<Option<StateTransition>, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let settle_match_call =
            settleMatchCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let maybe_party0_intent_creation_data = try_decode_intent_creation_data(
            recovery_id,
            &settle_match_call.party0SettlementBundle,
            &settle_match_call.obligationBundle,
            true, // is_party0
        )?;

        let maybe_party1_intent_creation_data = try_decode_intent_creation_data(
            recovery_id,
            &settle_match_call.party1SettlementBundle,
            &settle_match_call.obligationBundle,
            false, // is_party0
        )?;

        let maybe_intent_creation_data =
            maybe_party0_intent_creation_data.or(maybe_party1_intent_creation_data);

        // If we could not decode new intent shares for either party,
        // the registered recovery ID must not match the recovery ID of any
        // newly-created intents in the match.
        if maybe_intent_creation_data.is_none() {
            return Ok(None);
        }

        let intent_creation_data = maybe_intent_creation_data.unwrap();

        Ok(Some(StateTransition::CreateIntent(CreateIntentTransition {
            recovery_id,
            block_number,
            intent_creation_data,
        })))
    }

    /// Try to compute a `SettleMatchIntoIntent` state transition associated
    /// with the now-spent nullifier in a `settleMatch` call.
    ///
    /// Returns `None` if the spent nullifier does not match the intent
    /// nullifier of either party.
    async fn try_compute_settle_match_into_intent_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<Option<StateTransition>, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let settle_match_call =
            settleMatchCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let maybe_party0_intent_settlement_data = try_decode_intent_settlement_data(
            nullifier,
            &settle_match_call.party0SettlementBundle,
            &settle_match_call.obligationBundle,
            true, // is_party0
        )?;

        let maybe_party1_intent_settlement_data = try_decode_intent_settlement_data(
            nullifier,
            &settle_match_call.party1SettlementBundle,
            &settle_match_call.obligationBundle,
            false, // is_party0
        )?;

        let maybe_intent_settlement_data =
            maybe_party0_intent_settlement_data.or(maybe_party1_intent_settlement_data);

        // If we could not decode the updated intent amount share for either party,
        // the spent nullifier must pertain to one of the balances nullified in the
        // match.
        if maybe_intent_settlement_data.is_none() {
            return Ok(None);
        }

        let intent_settlement_data = maybe_intent_settlement_data.unwrap();

        Ok(Some(StateTransition::SettleMatchIntoIntent(SettleMatchIntoIntentTransition {
            nullifier,
            block_number,
            intent_settlement_data,
        })))
    }

    /// Compute a `CreatePublicIntent` state transition associated with
    /// the newly-created public intent (of the given hash) in a `settleMatch`
    /// call
    async fn compute_create_public_intent_transition(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let settle_match_call =
            settleMatchCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let party0_settlement_bundle_data: SettlementBundleData =
            (&settle_match_call.party0SettlementBundle).try_into()?;

        let party1_settlement_bundle_data: SettlementBundleData =
            (&settle_match_call.party1SettlementBundle).try_into()?;

        let maybe_party0_intent =
            party0_settlement_bundle_data.try_decode_public_intent(intent_hash)?;

        let maybe_party1_intent =
            party1_settlement_bundle_data.try_decode_public_intent(intent_hash)?;

        let intent = maybe_party0_intent.or(maybe_party1_intent).ok_or(
            IndexerError::invalid_settlement_bundle("no public intent found in settle match call"),
        )?;

        Ok(StateTransition::CreatePublicIntent(CreatePublicIntentTransition {
            intent,
            intent_hash,
            block_number,
        }))
    }

    /// Compute a `SettleMatchIntoPublicIntent` state transition associated with
    /// the settlement of a match into the given public intent in a
    /// `settleMatch` call
    async fn compute_settle_match_into_public_intent_transition(
        &self,
        intent_hash: B256,
        version: u64,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<StateTransition, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let settle_match_call =
            settleMatchCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let party0_settlement_bundle_data: SettlementBundleData =
            (&settle_match_call.party0SettlementBundle).try_into()?;

        let party1_settlement_bundle_data: SettlementBundleData =
            (&settle_match_call.party1SettlementBundle).try_into()?;

        let maybe_party0_intent =
            party0_settlement_bundle_data.try_decode_public_intent(intent_hash)?;

        let maybe_party1_intent =
            party1_settlement_bundle_data.try_decode_public_intent(intent_hash)?;

        let intent = maybe_party0_intent.or(maybe_party1_intent).ok_or(
            IndexerError::invalid_settlement_bundle("no public intent found in settle match call"),
        )?;

        Ok(StateTransition::SettleMatchIntoPublicIntent(SettleMatchIntoPublicIntentTransition {
            intent,
            intent_hash,
            version,
            block_number,
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
