//! Indexer-specific logic for indexing onchain events

use alloy::{
    hex,
    primitives::{TxHash, U256},
    sol_types::{SolCall, SolValue},
};
use renegade_circuit_types::{balance::BalanceShare, traits::BaseType};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::{
    ObligationBundle, PrivateIntentPublicBalanceBundle, PrivateIntentPublicBalanceFirstFillBundle,
    PrivateObligationBundle, PublicIntentPublicBalanceBundle, RenegadeSettledIntentBundle,
    RenegadeSettledIntentFirstFillBundle, RenegadeSettledPrivateFillBundle,
    RenegadeSettledPrivateFirstFillBundle, SettlementBundle, depositCall, depositNewBalanceCall,
    payFeesCall, settleMatchCall, withdrawCall,
};

use crate::{
    darkpool_client::utils::get_selector,
    indexer::{Indexer, error::IndexerError},
    state_transitions::{
        StateTransition, create_balance::CreateBalanceTransition, deposit::DepositTransition,
        pay_fees::PayFeesTransition, settle_match_into_balance::SettleMatchIntoBalanceTransition,
        withdraw::WithdrawTransition,
    },
};

// -------------
// | Constants |
// -------------

/// The value for the `NATIVELY_SETTLED_PUBLIC_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const NATIVELY_SETTLED_PUBLIC_INTENT: u8 = 0;
/// The value for the `NATIVELY_SETTLED_PRIVATE_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const NATIVELY_SETTLED_PRIVATE_INTENT: u8 = 1;
/// The value for the `RENEGADE_SETTLED_INTENT` variant of the Solidity
/// `SettlementBundleType` enum
const RENEGADE_SETTLED_INTENT: u8 = 2;
/// The value for the `RENEGADE_SETTLED_PRIVATE_FILL` variant of the Solidity
/// `SettlementBundleType` enum
const RENEGADE_SETTLED_PRIVATE_FILL: u8 = 3;

// ---------
// | Types |
// ---------

/// A wrapper around the different types of settlement bundle data
enum SettlementBundleData {
    /// A natively-settled, public-intent bundle
    PublicIntentPublicBalance(PublicIntentPublicBalanceBundle),
    /// A natively-settled, private-intent first fill bundle
    PrivateIntentPublicBalanceFirstFill(PrivateIntentPublicBalanceFirstFillBundle),
    /// A natively-settled, private-intent bundle
    PrivateIntentPublicBalance(PrivateIntentPublicBalanceBundle),
    /// A renegade-settled, public-fill intent first fill bundle
    RenegadeSettledIntentFirstFill(RenegadeSettledIntentFirstFillBundle),
    /// A renegade-settled, public-fill intent bundle
    RenegadeSettledIntent(RenegadeSettledIntentBundle),
    /// A renegade-settled, private-fill intent first fill bundle
    RenegadeSettledPrivateFirstFill(RenegadeSettledPrivateFirstFillBundle),
    /// A renegade-settled, private-fill intent bundle
    RenegadeSettledPrivateFill(RenegadeSettledPrivateFillBundle),
}

impl SettlementBundleData {
    /// Get the balance nullifier from the settlement bundle data, if one was
    /// spent
    pub fn get_balance_nullifier(&self) -> Option<Scalar> {
        let nullifier_u256 = match self {
            Self::RenegadeSettledIntentFirstFill(bundle) => bundle.auth.statement.balanceNullifier,
            Self::RenegadeSettledIntent(bundle) => bundle.auth.statement.balanceNullifier,
            Self::RenegadeSettledPrivateFirstFill(bundle) => bundle.auth.statement.balanceNullifier,
            Self::RenegadeSettledPrivateFill(bundle) => bundle.auth.statement.balanceNullifier,
            // Natively-settled bundles don't spend a balance state object's nullifier
            _ => return None,
        };

        Some(u256_to_scalar(&nullifier_u256))
    }

    /// Get the public shares for the new relayer fee, protocol fee, and amount
    /// in the private balance associated with this settlement bundle (if any).
    ///
    /// In the case of private-fill bundles, we parse the updated shares from
    /// the obligation bundle data.
    pub fn get_new_balance_public_shares(
        &self,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Option<(Scalar, Scalar, Scalar)>, IndexerError> {
        let [
            new_relayer_fee_public_share_u256,
            new_protocol_fee_public_share_u256,
            new_amount_public_share_u256,
        ] = match self {
            Self::RenegadeSettledIntentFirstFill(bundle) => {
                bundle.settlementStatement.newBalancePublicShares
            },
            Self::RenegadeSettledIntent(bundle) => {
                bundle.settlementStatement.newBalancePublicShares
            },
            Self::RenegadeSettledPrivateFirstFill(_) | Self::RenegadeSettledPrivateFill(_) => {
                decode_balance_shares_from_private_obligation_bundle(obligation_bundle, is_party0)?
            },
            // Natively-settled bundles don't update any balance state objects
            _ => return Ok(None),
        };

        Ok(Some((
            u256_to_scalar(&new_relayer_fee_public_share_u256),
            u256_to_scalar(&new_protocol_fee_public_share_u256),
            u256_to_scalar(&new_amount_public_share_u256),
        )))
    }
}

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
                self.compute_deposit_state_transition(nullifier, tx_hash, &calldata).await
            },
            withdrawCall::SELECTOR => {
                self.compute_withdraw_state_transition(nullifier, tx_hash, &calldata).await
            },
            payFeesCall::SELECTOR => {
                self.compute_pay_fees_state_transition(nullifier, tx_hash, &calldata).await
            },
            settleMatchCall::SELECTOR => {
                let maybe_settle_match_into_balance_transition = self
                    .try_compute_settle_match_into_balance_state_transition(
                        nullifier, tx_hash, &calldata,
                    )
                    .await?;

                // TODO: Implement getting intent settlement transition
                let maybe_settle_match_into_intent_transition = None;

                maybe_settle_match_into_balance_transition
                    .xor(maybe_settle_match_into_intent_transition)
                    .ok_or(IndexerError::invalid_settlement_bundle(
                        "no balance or intent nullified in match tx 0x{tx_hash:#x}",
                    ))
            },
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

    /// Try to compute a `SettleMatchIntoBalance` state transition associated
    /// with the now-spent nullifier in a `settleMatch` call.
    ///
    /// Returns `None` if the spent nullifier does not match the balance
    /// nullifier of either party.
    async fn try_compute_settle_match_into_balance_state_transition(
        &self,
        nullifier: Scalar,
        tx_hash: TxHash,
        calldata: &[u8],
    ) -> Result<Option<StateTransition>, IndexerError> {
        let block_number = self.darkpool_client.get_tx_block_number(tx_hash).await?;

        let settle_match_call =
            settleMatchCall::abi_decode(calldata).map_err(IndexerError::parse)?;

        let maybe_party0_balance_shares = try_decode_balance_shares_for_party(
            nullifier,
            &settle_match_call.party0SettlementBundle,
            &settle_match_call.obligationBundle,
            true, // is_party0
        )?;

        let maybe_party1_balance_shares = try_decode_balance_shares_for_party(
            nullifier,
            &settle_match_call.party1SettlementBundle,
            &settle_match_call.obligationBundle,
            false, // is_party0
        )?;

        // If we could not decode balance shares for either party,
        // the spent nullifier must pertain to one of the intents nullified in the
        // match.
        if maybe_party0_balance_shares.is_none() && maybe_party1_balance_shares.is_none() {
            return Ok(None);
        }

        let (new_relayer_fee_public_share, new_protocol_fee_public_share, new_amount_public_share) =
            maybe_party0_balance_shares
                .xor(maybe_party1_balance_shares)
                .ok_or(IndexerError::invalid_settlement_bundle("no new balance public shares"))?;

        Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
            nullifier,
            block_number,
            new_relayer_fee_public_share,
            new_protocol_fee_public_share,
            new_amount_public_share,
        })))
    }
}

// -----------
// | Helpers |
// -----------

/// Try to decode the new balance public shares from the match party's
/// settlement bundle & obligation bundle.
///
/// Returns `None` if the spent nullifier does not match the party's balance
/// nullifier.
fn try_decode_balance_shares_for_party(
    nullifier: Scalar,
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<Option<(Scalar, Scalar, Scalar)>, IndexerError> {
    let settlement_bundle_data = decode_settlement_bundle_data(settlement_bundle)?;

    let balance_nullified = settlement_bundle_data.get_balance_nullifier() == Some(nullifier);

    if !balance_nullified {
        return Ok(None);
    }

    settlement_bundle_data.get_new_balance_public_shares(obligation_bundle, is_party0)
}

/// Decode the settlement bundle data for a renegade-settled bundle.
///
/// Returns `None` if the bundle is a natively-settled bundle.
fn decode_settlement_bundle_data(
    settlement_bundle: &SettlementBundle,
) -> Result<SettlementBundleData, IndexerError> {
    let is_first_fill = settlement_bundle.isFirstFill;
    let bundle_type = settlement_bundle.bundleType;

    match bundle_type {
        // Natively-settled bundles don't spend a balance state object's nullifier
        NATIVELY_SETTLED_PUBLIC_INTENT => {
            PublicIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)
                .map(SettlementBundleData::PublicIntentPublicBalance)
        },
        NATIVELY_SETTLED_PRIVATE_INTENT => {
            if is_first_fill {
                PrivateIntentPublicBalanceFirstFillBundle::abi_decode(&settlement_bundle.data)
                    .map_err(IndexerError::parse)
                    .map(SettlementBundleData::PrivateIntentPublicBalanceFirstFill)
            } else {
                PrivateIntentPublicBalanceBundle::abi_decode(&settlement_bundle.data)
                    .map_err(IndexerError::parse)
                    .map(SettlementBundleData::PrivateIntentPublicBalance)
            }
        },
        RENEGADE_SETTLED_INTENT => {
            if is_first_fill {
                RenegadeSettledIntentFirstFillBundle::abi_decode(&settlement_bundle.data)
                    .map_err(IndexerError::parse)
                    .map(SettlementBundleData::RenegadeSettledIntentFirstFill)
            } else {
                RenegadeSettledIntentBundle::abi_decode(&settlement_bundle.data)
                    .map_err(IndexerError::parse)
                    .map(SettlementBundleData::RenegadeSettledIntent)
            }
        },
        RENEGADE_SETTLED_PRIVATE_FILL => {
            if is_first_fill {
                RenegadeSettledPrivateFirstFillBundle::abi_decode(&settlement_bundle.data)
                    .map_err(IndexerError::parse)
                    .map(SettlementBundleData::RenegadeSettledPrivateFirstFill)
            } else {
                RenegadeSettledPrivateFillBundle::abi_decode(&settlement_bundle.data)
                    .map_err(IndexerError::parse)
                    .map(SettlementBundleData::RenegadeSettledPrivateFill)
            }
        },
        _ => Err(IndexerError::invalid_settlement_bundle(format!(
            "invalid settlement bundle type: {bundle_type}"
        ))),
    }
}

/// Decode the given party's new balance public shares from the given obligation
/// bundle, assuming it is a private obligation bundle.
fn decode_balance_shares_from_private_obligation_bundle(
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
) -> Result<[U256; 3], IndexerError> {
    let private_obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
        .map_err(IndexerError::parse)?;

    if is_party0 {
        Ok(private_obligation_bundle.statement.party0NewBalancePublicShares)
    } else {
        Ok(private_obligation_bundle.statement.party1NewBalancePublicShares)
    }
}
