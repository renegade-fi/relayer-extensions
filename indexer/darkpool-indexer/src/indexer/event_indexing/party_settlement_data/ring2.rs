//! Utilities for constructing & interacting with ring 2 settlement data

use alloy::{
    primitives::{Address, TxHash},
    sol_types::SolValue,
};
use renegade_circuit_types::{
    Nullifier,
    balance::{PostMatchBalanceShare, PreMatchBalanceShare},
    fixed_point::FixedPoint,
    intent::{IntentShare, PreMatchIntentShare},
    settlement_obligation::SettlementObligation as CircuitSettlementObligation,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::{
    IDarkpoolV2::{
        ExistingBalanceBundle, NewBalanceBundle, ObligationBundle, RenegadeSettledIntentBundle,
        RenegadeSettledIntentFirstFillBundle, SettlementBundle, SettlementObligation,
    },
    calldata_bundles::{EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE, NEW_OUTPUT_BALANCE_BUNDLE_TYPE},
};

use crate::{
    darkpool_client::DarkpoolClient,
    indexer::{
        error::IndexerError,
        event_indexing::party_settlement_data::{
            PartySettlementData, parse_party_settlement_obligation,
        },
    },
    state_transitions::{
        StateTransition,
        create_balance::{BalanceCreationData, CreateBalanceTransition},
        create_intent::{CreateIntentTransition, IntentCreationData},
        settle_match_into_balance::{BalanceSettlementData, SettleMatchIntoBalanceTransition},
        settle_match_into_intent::{IntentSettlementData, SettleMatchIntoIntentTransition},
    },
};

// ---------
// | Types |
// ---------

/// Settlement data for a ring 2 (renegade-settled, public-fill) settlement
/// representing the first fill on the party's intent, into a new output balance
pub struct Ring2FirstFillNewOutBalanceSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledIntentFirstFillBundle,
    /// The new output balance bundle
    pub new_balance_bundle: NewBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

// --- Public API ---
impl Ring2FirstFillNewOutBalanceSettlementData {
    /// Parse ring 2 first fill new output balance bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledIntentFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        let new_balance_bundle =
            NewBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        Ok(Ring2FirstFillNewOutBalanceSettlementData {
            settlement_bundle: settlement_bundle_data,
            new_balance_bundle,
            settlement_obligation,
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
            let relayer_fee_rate = self.get_relayer_fee_rate();
            let settlement_obligation = self.get_settlement_obligation();

            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

            let protocol_fee_rate =
                self.get_protocol_fee_rate(darkpool_client, block_number).await?;

            let balance_creation_data = BalanceCreationData::NewOutputBalanceFromPublicFill {
                pre_match_balance_share,
                post_match_balance_share,
                settlement_obligation,
                relayer_fee_rate,
                protocol_fee_rate,
            };

            Ok(Some(StateTransition::CreateBalance(CreateBalanceTransition {
                recovery_id,
                block_number,
                balance_creation_data,
            })))
        } else if self.get_new_intent_recovery_id() == recovery_id {
            // If the recovery ID matches that of the newly-created intent, we produce a
            // create intent transition

            let pre_match_full_intent_share = self.get_intent_share();
            let settlement_obligation = self.get_settlement_obligation();

            let intent_creation_data = IntentCreationData::PublicFill {
                pre_match_full_intent_share,
                settlement_obligation,
            };

            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

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

        let settlement_obligation = self.get_settlement_obligation();
        let new_one_time_authority_share = self.get_new_one_time_authority_share();

        let balance_settlement_data = BalanceSettlementData::PublicFirstFillInputBalance {
            settlement_obligation,
            new_one_time_authority_share,
        };

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

        Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
            nullifier,
            block_number,
            balance_settlement_data,
        })))
    }
}

// -- Private Helpers ---
impl Ring2FirstFillNewOutBalanceSettlementData {
    /// Get the new output balance recovery ID from the settlement bundle data
    fn get_new_output_balance_recovery_id(&self) -> Scalar {
        u256_to_scalar(&self.new_balance_bundle.statement.recoveryId)
    }

    /// Get the newly-created intent's recovery ID from the settlement bundle
    /// data
    fn get_new_intent_recovery_id(&self) -> Scalar {
        u256_to_scalar(&self.settlement_bundle.auth.statement.intentRecoveryId)
    }

    /// Get the public sharing of the new output balance fields which are not
    /// affected by the match
    fn get_pre_match_output_balance_share(&self) -> PreMatchBalanceShare {
        self.new_balance_bundle.statement.preMatchBalanceShares.clone().into()
    }

    /// Get the public sharing of the pre-update new output balance fields which
    /// are affected by the match
    fn get_post_match_output_balance_share(&self) -> PostMatchBalanceShare {
        self.settlement_bundle.settlementStatement.outBalancePublicShares.clone().into()
    }

    /// Get the relayer fee rate from the settlement bundle data
    fn get_relayer_fee_rate(&self) -> FixedPoint {
        self.settlement_bundle.settlementStatement.relayerFee.clone().into()
    }

    /// Get the protocol fee rate for the traded pair at the given block number
    async fn get_protocol_fee_rate(
        &self,
        darkpool_client: &DarkpoolClient,
        block_number: u64,
    ) -> Result<FixedPoint, IndexerError> {
        let asset0 = self.settlement_obligation.inputToken;
        let asset1 = self.settlement_obligation.outputToken;

        darkpool_client
            .get_protocol_fee_rate_at_block(asset0, asset1, block_number)
            .await
            .map_err(IndexerError::rpc)
    }

    /// Get the settlement obligation
    fn get_settlement_obligation(&self) -> CircuitSettlementObligation {
        self.settlement_obligation.clone().into()
    }

    /// Get the pre-update intent share from the settlement bundle data
    fn get_intent_share(&self) -> IntentShare {
        let PreMatchIntentShare { in_token, out_token, owner, min_price } =
            self.settlement_bundle.auth.statement.intentPublicShare.clone().into();

        let amount_in =
            u256_to_scalar(&self.settlement_bundle.settlementStatement.amountPublicShare);

        IntentShare { in_token, out_token, owner, min_price, amount_in }
    }

    /// Get the spent input balance nullifier from the settlement bundle data
    fn get_input_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.settlement_bundle.auth.statement.oldBalanceNullifier)
    }

    /// Get the new one-time authority share from the settlement bundle data
    fn get_new_one_time_authority_share(&self) -> Scalar {
        u256_to_scalar(&self.settlement_bundle.auth.statement.newOneTimeAddressPublicShare)
    }
}

/// Settlement data for a ring 2 (renegade-settled, public-fill) settlement
/// representing the first fill on the party's intent, into an existing output
/// balance
pub struct Ring2FirstFillSettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledIntentFirstFillBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

// --- Public API ---
impl Ring2FirstFillSettlementData {
    /// Parse ring 2 first fill bundle data from the given
    /// settlement & obligation bundles. Expects the settlement bundle data
    /// to already have been decoded.
    pub fn new(
        settlement_bundle_data: RenegadeSettledIntentFirstFillBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        Ok(Ring2FirstFillSettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            settlement_obligation,
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

        let pre_match_full_intent_share = self.get_intent_share();
        let settlement_obligation = self.get_settlement_obligation();

        let intent_creation_data =
            IntentCreationData::PublicFill { pre_match_full_intent_share, settlement_obligation };

        let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

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
        if self.get_input_balance_nullifier() == nullifier {
            let settlement_obligation = self.get_settlement_obligation();
            let new_one_time_authority_share = self.get_new_one_time_authority_share();

            let balance_settlement_data = BalanceSettlementData::PublicFirstFillInputBalance {
                settlement_obligation,
                new_one_time_authority_share,
            };

            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

            Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
                nullifier,
                block_number,
                balance_settlement_data,
            })))
        } else if self.get_output_balance_nullifier() == nullifier {
            let settlement_obligation = self.get_settlement_obligation();
            let relayer_fee_rate = self.get_relayer_fee_rate();

            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

            let (asset0, asset1) = self.get_trading_pair();
            let protocol_fee_rate = darkpool_client
                .get_protocol_fee_rate_at_block(asset0, asset1, block_number)
                .await
                .map_err(IndexerError::rpc)?;

            let balance_settlement_data = BalanceSettlementData::PublicFillOutputBalance {
                settlement_obligation,
                relayer_fee_rate,
                protocol_fee_rate,
            };

            Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
                nullifier,
                block_number,
                balance_settlement_data,
            })))
        } else {
            Ok(None)
        }
    }
}

// -- Private Helpers ---
impl Ring2FirstFillSettlementData {
    /// Get the newly-created intent's recovery ID from the settlement bundle
    /// data
    fn get_new_intent_recovery_id(&self) -> Scalar {
        u256_to_scalar(&self.settlement_bundle.auth.statement.intentRecoveryId)
    }

    /// Get the pre-update intent share from the settlement bundle data
    fn get_intent_share(&self) -> IntentShare {
        let PreMatchIntentShare { in_token, out_token, owner, min_price } =
            self.settlement_bundle.auth.statement.intentPublicShare.clone().into();

        let amount_in =
            u256_to_scalar(&self.settlement_bundle.settlementStatement.amountPublicShare);

        IntentShare { in_token, out_token, owner, min_price, amount_in }
    }

    /// Get the settlement obligation
    fn get_settlement_obligation(&self) -> CircuitSettlementObligation {
        self.settlement_obligation.clone().into()
    }

    /// Get the spent input balance nullifier from the settlement bundle data
    fn get_input_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.settlement_bundle.auth.statement.oldBalanceNullifier)
    }

    /// Get the spent output balance nullifier from the settlement bundle data
    fn get_output_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.existing_balance_bundle.statement.oldBalanceNullifier)
    }

    /// Get the new one-time authority share from the settlement bundle data
    fn get_new_one_time_authority_share(&self) -> Scalar {
        u256_to_scalar(&self.settlement_bundle.auth.statement.newOneTimeAddressPublicShare)
    }

    /// Get the relayer fee rate from the settlement bundle data
    fn get_relayer_fee_rate(&self) -> FixedPoint {
        self.settlement_bundle.settlementStatement.relayerFee.clone().into()
    }

    /// Get the asset pair traded in this match
    fn get_trading_pair(&self) -> (Address, Address) {
        (self.settlement_obligation.inputToken, self.settlement_obligation.outputToken)
    }
}

/// Settlement data for a ring 2 (renegade-settled, public-fill) settlement that
/// was not the first fill on the party's intent
pub struct Ring2SettlementData {
    /// The settlement bundle data
    pub settlement_bundle: RenegadeSettledIntentBundle,
    /// The existing output balance bundle
    pub existing_balance_bundle: ExistingBalanceBundle,
    /// The settlement obligation
    pub settlement_obligation: SettlementObligation,
}

// --- Public API ---
impl Ring2SettlementData {
    /// Parse ring 2 bundle data from the given settlement & obligation bundles.
    pub fn new(
        settlement_bundle: &SettlementBundle,
        obligation_bundle: &ObligationBundle,
        is_party0: bool,
    ) -> Result<Self, IndexerError> {
        let settlement_bundle_data =
            RenegadeSettledIntentBundle::abi_decode(&settlement_bundle.data)
                .map_err(IndexerError::parse)?;

        let existing_balance_bundle =
            ExistingBalanceBundle::abi_decode(&settlement_bundle_data.outputBalanceBundle.data)
                .map_err(IndexerError::parse)?;

        let settlement_obligation =
            parse_party_settlement_obligation(obligation_bundle, is_party0)?;

        Ok(Ring2SettlementData {
            settlement_bundle: settlement_bundle_data,
            existing_balance_bundle,
            settlement_obligation,
        })
    }

    /// Get the state transition associated with the nullifier spend event.
    ///
    /// Returns `None` if the nullifier doesn't match any of this party's
    /// spent nullifiers.
    pub async fn get_state_transition_for_nullifier(
        &self,
        darkpool_client: &DarkpoolClient,
        nullifier: Nullifier,
        tx_hash: TxHash,
    ) -> Result<Option<StateTransition>, IndexerError> {
        if self.get_input_balance_nullifier() == nullifier {
            let settlement_obligation = self.get_settlement_obligation();
            let balance_settlement_data =
                BalanceSettlementData::PublicFillInputBalance { settlement_obligation };
            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

            Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
                nullifier,
                block_number,
                balance_settlement_data,
            })))
        } else if self.get_output_balance_nullifier() == nullifier {
            let settlement_obligation = self.get_settlement_obligation();
            let relayer_fee_rate = self.get_relayer_fee_rate();

            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

            let (asset0, asset1) = self.get_trading_pair();
            let protocol_fee_rate = darkpool_client
                .get_protocol_fee_rate_at_block(asset0, asset1, block_number)
                .await
                .map_err(IndexerError::rpc)?;

            let balance_settlement_data = BalanceSettlementData::PublicFillOutputBalance {
                settlement_obligation,
                relayer_fee_rate,
                protocol_fee_rate,
            };

            Ok(Some(StateTransition::SettleMatchIntoBalance(SettleMatchIntoBalanceTransition {
                nullifier,
                block_number,
                balance_settlement_data,
            })))
        } else if self.get_intent_nullifier() == nullifier {
            let settlement_obligation = self.get_settlement_obligation();
            let intent_settlement_data = IntentSettlementData::PublicFill { settlement_obligation };
            let block_number = darkpool_client.get_tx_block_number(tx_hash).await?;

            Ok(Some(StateTransition::SettleMatchIntoIntent(SettleMatchIntoIntentTransition {
                nullifier,
                block_number,
                intent_settlement_data,
            })))
        } else {
            Ok(None)
        }
    }
}

// --- Private Helpers
impl Ring2SettlementData {
    /// Get the spent input balance nullifier from the settlement bundle data
    fn get_input_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.settlement_bundle.auth.statement.oldBalanceNullifier)
    }

    /// Get the spent output balance nullifier from the settlement bundle data
    fn get_output_balance_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.existing_balance_bundle.statement.oldBalanceNullifier)
    }

    /// Get the spent intent nullifier from the settlement bundle data
    fn get_intent_nullifier(&self) -> Nullifier {
        u256_to_scalar(&self.settlement_bundle.auth.statement.oldIntentNullifier)
    }

    /// Get the settlement obligation
    fn get_settlement_obligation(&self) -> CircuitSettlementObligation {
        self.settlement_obligation.clone().into()
    }

    /// Get the relayer fee rate from the settlement bundle data
    fn get_relayer_fee_rate(&self) -> FixedPoint {
        self.settlement_bundle.settlementStatement.relayerFee.clone().into()
    }

    /// Get the asset pair traded in this match
    fn get_trading_pair(&self) -> (Address, Address) {
        (self.settlement_obligation.inputToken, self.settlement_obligation.outputToken)
    }
}

// -------------------
// | Parsing Helpers |
// -------------------

/// Parse ring 2 settlement data from the given settlement & obligation bundles
pub fn parse_ring2_settlement_data(
    settlement_bundle: &SettlementBundle,
    obligation_bundle: &ObligationBundle,
    is_party0: bool,
    is_first_fill: bool,
) -> Result<PartySettlementData, IndexerError> {
    if !is_first_fill {
        return Ring2SettlementData::new(settlement_bundle, obligation_bundle, is_party0)
            .map(PartySettlementData::Ring2);
    }

    let settlement_bundle_data =
        RenegadeSettledIntentFirstFillBundle::abi_decode(&settlement_bundle.data)
            .map_err(IndexerError::parse)?;

    let output_bundle_type = settlement_bundle_data.outputBalanceBundle.bundleType;

    match output_bundle_type {
        EXISTING_OUTPUT_BALANCE_BUNDLE_TYPE => {
            Ring2FirstFillSettlementData::new(settlement_bundle_data, obligation_bundle, is_party0)
                .map(PartySettlementData::Ring2FirstFill)
        },
        NEW_OUTPUT_BALANCE_BUNDLE_TYPE => Ring2FirstFillNewOutBalanceSettlementData::new(
            settlement_bundle_data,
            obligation_bundle,
            is_party0,
        )
        .map(PartySettlementData::Ring2FirstFillNewOutBalance),
        _ => Err(IndexerError::invalid_output_balance_bundle(format!(
            "invalid output balance bundle type: {}",
            output_bundle_type
        ))),
    }
}
