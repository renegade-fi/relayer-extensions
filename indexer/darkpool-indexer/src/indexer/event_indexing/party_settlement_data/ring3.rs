//! Utilities for constructing & interacting with ring 3 settlement data

use alloy::{primitives::TxHash, sol_types::SolValue};
use renegade_circuit_types::{
    Nullifier,
    balance::{PostMatchBalanceShare, PreMatchBalanceShare},
    intent::{IntentShare, PreMatchIntentShare},
};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ExistingBalanceBundle, NewBalanceBundle, ObligationBundle, PrivateObligationBundle,
        RenegadeSettledPrivateFillBundle, RenegadeSettledPrivateFirstFillBundle, SettlementBundle,
    },
    calldata_bundles::{EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE, NEW_OUTPUT_BALANCE_BUNDLE_TYPE},
};

use crate::{
    darkpool_client::DarkpoolClient,
    indexer::{error::IndexerError, event_indexing::party_settlement_data::PartySettlementData},
    state_transitions::{
        StateTransition,
        create_balance::{BalanceCreationData, CreateBalanceTransition},
        create_intent::{CreateIntentTransition, IntentCreationData},
        settle_match_into_balance::{BalanceSettlementData, SettleMatchIntoBalanceTransition},
    },
};

// ---------
// | Types |
// ---------

/// Settlement data for a ring 3 (renegade-settled, private-fill) settlement
/// representing the first fill on the party's intent, into a new output balance
pub struct Ring3FirstFillNewOutBalanceSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledPrivateFirstFillBundle,
    /// The new output balance bundle
    pub new_balance_bundle: NewBalanceBundle,
    /// The private obligation bundle
    pub obligation_bundle: PrivateObligationBundle,
    /// Whether the party is party 0
    pub is_party0: bool,
}

// --- Public API ---
impl Ring3FirstFillNewOutBalanceSettlementData {
    /// Parse ring 3 first fill new output balance bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledPrivateFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let new_balance_bundle =
            NewBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
            .map_err(IndexerError::parse)?;

        Ok(Ring3FirstFillNewOutBalanceSettlementData {
            settlement_bundle: settlement_bundle_data,
            new_balance_bundle,
            obligation_bundle,
            is_party0,
        })
    }

    /// Get the state transition associated with the recovery ID event.
    ///
    /// Returns `None` if this party did not create a state object with the
    /// given recovery ID.
    pub async fn get_state_transition_for_recovery_id(
        &self,
        darkpool_client: &DarkpoolClient,
        recovery_id: Scalar,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        if self.get_new_output_balance_recovery_id() == recovery_id {
            // If the recovery ID matches that of the newly-created output balance, we
            // produce a create balance transition
            let pre_match_balance_share = self.get_pre_match_output_balance_share();
            let post_match_balance_share = self.get_post_match_output_balance_share();

            let balance_creation_data = BalanceCreationData::NewOutputBalanceFromPrivateFill {
                pre_match_balance_share,
                post_match_balance_share,
            };

            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

            Ok(Some(StateTransition::CreateBalance(CreateBalanceTransition {
                recovery_id,
                block_number,
                balance_creation_data,
            })))
        } else if self.get_new_intent_recovery_id() == recovery_id {
            // If the recovery ID matches that of the newly-created intent, we produce a
            // create intent transition

            let intent_share = self.get_intent_share();
            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;
            let intent_creation_data = IntentCreationData::RenegadeSettledPrivateFill(intent_share);

            Ok(Some(StateTransition::CreateIntent(CreateIntentTransition {
                recovery_id,
                block_number,
                intent_creation_data,
            })))
        } else {
            Ok(None)
        }
    }

    /// Get the state transition associated with the nullifier spend event.
    ///
    /// Returns `None` if the nullifier doesn't match this party's spent input
    /// balance nullifier.
    pub async fn get_state_transition_for_nullifier(
        &self,
        darkpool_client: &DarkpoolClient,
        nullifier: Nullifier,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the given nullifier doesn't match the input balance nullifier spent by
        // this party, we don't produce a state transition for this event.
        if self.get_input_balance_nullifier() != nullifier {
            return Ok(None);
        }

        let post_match_balance_share = self.get_post_match_input_balance_share();
        let balance_settlement_data = BalanceSettlementData::PrivateFill(post_match_balance_share);

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
            nullifier,
            block_number,
            balance_settlement_data,
        })))
    }
}

// --- Private Helpers ---
impl Ring3FirstFillNewOutBalanceSettlementData {
    /// Get the new output balance recovery ID
    fn get_new_output_balance_recovery_id(&self) -> Scalar {
        u256_to_scalar(&self.new_balance_bundle.statement.recoveryId)
    }

    /// Get the newly-created intent's recovery ID
    fn get_new_intent_recovery_id(&self) -> Scalar {
        u256_to_scalar(&self.settlement_bundle.auth.statement.intentRecoveryId)
    }

    /// Get the public sharing of the new output balance fields which are not
    /// affected by the match
    fn get_pre_match_output_balance_share(&self) -> PreMatchBalanceShare {
        self.new_balance_bundle.statement.preMatchBalanceShares.clone().into()
    }

    /// Get the public sharing of the post-update new output balance fields
    /// which are affected by the match
    fn get_post_match_output_balance_share(&self) -> PostMatchBalanceShare {
        if self.is_party0 {
            self.obligation_bundle.statement.newOutBalancePublicShares0.clone().into()
        } else {
            self.obligation_bundle.statement.newOutBalancePublicShares1.clone().into()
        }
    }

    /// Get the post-update intent share
    fn get_intent_share(&self) -> IntentShare {
        let PreMatchIntentShare { in_token, out_token, owner, min_price } =
            self.settlement_bundle.auth.statement.intentPublicShare.clone().into();

        let amount_in_u256 = if self.is_party0 {
            &self.obligation_bundle.statement.newAmountPublicShare0
        } else {
            &self.obligation_bundle.statement.newAmountPublicShare1
        };

        let amount_in = u256_to_scalar(amount_in_u256);

        IntentShare { in_token, out_token, owner, min_price, amount_in }
    }

    /// Get the spent input balance nullifier
    fn get_input_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.settlement_bundle.auth.statement.oldBalanceNullifier)
    }

    /// Get the public sharing of the post-update input balance fields which are
    /// affected by the match
    fn get_post_match_input_balance_share(&self) -> PostMatchBalanceShare {
        if self.is_party0 {
            self.obligation_bundle.statement.newInBalancePublicShares0.clone().into()
        } else {
            self.obligation_bundle.statement.newInBalancePublicShares1.clone().into()
        }
    }
}

/// Settlement data for a ring 3 (renegade-settled, private-fill) settlement
/// representing the first fill on the party's intent, into an existing output
/// balance
pub struct Ring3FirstFillSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledPrivateFirstFillBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The private obligation bundle
    pub obligation_bundle: PrivateObligationBundle,
    /// Whether the party is party 0
    pub is_party0: bool,
}

// --- Public API ---
impl Ring3FirstFillSettlementData {
    /// Parse ring 3 first fill bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledPrivateFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
            .map_err(IndexerError::parse)?;

        Ok(Ring3FirstFillSettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            obligation_bundle,
            is_party0,
        })
    }

    /// Get the state transition associated with the recovery ID event.
    ///
    /// Returns `None` if this party's newly-created intent doesn't have the
    /// given recovery ID.
    pub async fn get_state_transition_for_recovery_id(
        &self,
        darkpool_client: &DarkpoolClient,
        recovery_id: Scalar,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        // If the given recovery ID doesn't match that of the newly-created intent
        // in this bundle, we don't produce a state transition for this event.
        if self.get_new_intent_recovery_id() != recovery_id {
            return Ok(None);
        }

        let intent_share = self.get_intent_share();
        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;
        let intent_creation_data = IntentCreationData::RenegadeSettledPrivateFill(intent_share);

        Ok(Some(StateTransition::CreateIntent(CreateIntentTransition {
            recovery_id,
            block_number,
            intent_creation_data,
        })))
    }

    /// Get the state transition associated with the nullifier spend event.
    ///
    /// Returns `None` if the nullifier doesn't match either of this party's
    /// spent balance nullifiers.
    pub async fn get_state_transition_for_nullifier(
        &self,
        darkpool_client: &DarkpoolClient,
        nullifier: Nullifier,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        let post_match_balance_share = if self.get_input_balance_nullifier() == nullifier {
            self.get_post_match_input_balance_share()
        } else if self.get_output_balance_nullifier() == nullifier {
            self.get_post_match_output_balance_share()
        } else {
            // If neither the input nor output balance nullifiers match the given nullifier,
            // we don't produce a state transition for this event.
            return Ok(None);
        };

        let balance_settlement_data = BalanceSettlementData::PrivateFill(post_match_balance_share);

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
            nullifier,
            block_number,
            balance_settlement_data,
        })))
    }
}

// --- Private Helpers ---
impl Ring3FirstFillSettlementData {
    /// Get the newly-created intent's recovery ID
    fn get_new_intent_recovery_id(&self) -> Scalar {
        u256_to_scalar(&self.settlement_bundle.auth.statement.intentRecoveryId)
    }

    /// Get the post-update intent share
    fn get_intent_share(&self) -> IntentShare {
        let PreMatchIntentShare { in_token, out_token, owner, min_price } =
            self.settlement_bundle.auth.statement.intentPublicShare.clone().into();

        let amount_in_u256 = if self.is_party0 {
            &self.obligation_bundle.statement.newAmountPublicShare0
        } else {
            &self.obligation_bundle.statement.newAmountPublicShare1
        };

        let amount_in = u256_to_scalar(amount_in_u256);

        IntentShare { in_token, out_token, owner, min_price, amount_in }
    }

    /// Get the spent input balance nullifier
    fn get_input_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.settlement_bundle.auth.statement.oldBalanceNullifier)
    }

    /// Get the spent output balance nullifier
    fn get_output_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.existing_balance_bundle.statement.oldBalanceNullifier)
    }

    /// Get the public sharing of the post-update input balance fields which are
    /// affected by the match
    fn get_post_match_input_balance_share(&self) -> PostMatchBalanceShare {
        if self.is_party0 {
            self.obligation_bundle.statement.newInBalancePublicShares0.clone().into()
        } else {
            self.obligation_bundle.statement.newInBalancePublicShares1.clone().into()
        }
    }

    /// Get the public sharing of the post-update output balance fields which
    /// are affected by the match
    fn get_post_match_output_balance_share(&self) -> PostMatchBalanceShare {
        if self.is_party0 {
            self.obligation_bundle.statement.newOutBalancePublicShares0.clone().into()
        } else {
            self.obligation_bundle.statement.newOutBalancePublicShares1.clone().into()
        }
    }
}

/// Settlement data for a ring 3 (renegade-settled, private-fill) settlement
/// that was not the first fill on the party's intent
pub struct Ring3SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledPrivateFillBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The private obligation bundle
    pub obligation_bundle: PrivateObligationBundle,
    /// Whether the party is party 0
    pub is_party0: bool,
}

impl Ring3SettlementData {
    /// Parse ring 3 bundle data from the given settlement & obligation bundles.
    pub fn new(
        settlement_bundle: &SettlementBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            RenegadeSettledPrivateFillBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let obligation_bundle = PrivateObligationBundle::abi_decode(&obligation_bundle.data)
            .map_err(IndexerError::parse)?;

        Ok(Ring3SettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            obligation_bundle,
            is_party0,
        })
    }
}

// -------------------
// | Parsing Helpers |
// -------------------

/// Parse ring 3 settlement data from the given settlement & obligation bundles
pub fn parse_ring3_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
    is_first_fill: bool,
) -> Result<PartySettlementData, IndexerError> {
    if !is_first_fill {
        return Ring3SettlementData::new(settlement_bundle, obligation_bundle, is_party0)
            .map(PartySettlementData::Ring3);
    }

    let settlement_bundle_data =
        RenegadeSettledPrivateFirstFillBundle::abi_decode(&settlement_bundle.data)
            .map_err(IndexerError::parse)?;

    let output_bundle_type = settlement_bundle_data.outputBalanceBundle.bundleType;

    match output_bundle_type {
        EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE => {
            Ring3FirstFillSettlementData::new(settlement_bundle_data, obligation_bundle, is_party0)
                .map(PartySettlementData::Ring3FirstFill)
        },
        NEW_OUTPUT_BALANCE_BUNDLE_TYPE => Ring3FirstFillNewOutBalanceSettlementData::new(
            settlement_bundle_data,
            obligation_bundle,
            is_party0,
        )
        .map(PartySettlementData::Ring3FirstFillNewOutBalance),
        _ => Err(IndexerError::invalid_output_balance_bundle(format!(
            "invalid output balance bundle type: {}",
            output_bundle_type
        ))),
    }
}
