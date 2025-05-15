//! Phase one of the sweeper's execution; index all fees since the last
//! consistent block

use alloy::consensus::constants::SELECTOR_LEN;
use alloy::consensus::Transaction;
use alloy::providers::Provider;
use alloy::rpc::types::Log;
use alloy_primitives::TxHash;
use alloy_sol_types::SolCall;
use renegade_circuit_types::elgamal::{BabyJubJubPoint, DecryptionKey, ElGamalCiphertext};
use renegade_circuit_types::native_helpers::elgamal_decrypt;
use renegade_circuit_types::note::{Note, NOTE_CIPHERTEXT_SIZE};
use renegade_circuit_types::wallet::NoteCommitment;
use renegade_constants::Scalar;
use renegade_crypto::fields::{scalar_to_biguint, scalar_to_u128};
use renegade_darkpool_client::{
    arbitrum::{
        abi::Darkpool::{settleOfflineFeeCall, NotePosted},
        contract_types::types::ValidOfflineFeeSettlementStatement as ContractValidOfflineFeeSettlementStatement,
        helpers::deserialize_calldata,
    },
    conversion::u256_to_scalar,
};
use renegade_util::err_str;
use tracing::{info, warn};

use crate::db::models::NewFee;
use crate::error::FundsManagerError;
use crate::Indexer;

/// Error message for when a tx is not found
const ERR_TX_NOT_FOUND: &str = "tx not found";
/// Error message for when a block number is not found for a given event
const ERR_NO_BLOCK_NUMBER: &str = "block number not found";
/// Error message for when a tx hash is not found for a given event
const ERR_NO_TX_HASH: &str = "tx hash not found";
/// Error message for when a failed to create a note posted stream
const ERR_FAILED_TO_CREATE_NOTE_POSTED_STREAM: &str = "failed to create note posted stream";

impl Indexer {
    /// Index all fees since the given block
    pub async fn index_fees(&self) -> Result<(), FundsManagerError> {
        let block_number = self.get_latest_block().await?;
        info!("indexing fees from block {block_number}");

        let filter = self.darkpool_client.event_filter::<NotePosted>().from_block(block_number);

        let events = filter
            .query()
            .await
            .map_err(|_| FundsManagerError::arbitrum(ERR_FAILED_TO_CREATE_NOTE_POSTED_STREAM))?;

        let mut most_recent_block = block_number;
        for (event, meta) in events {
            let block =
                meta.block_number.ok_or(FundsManagerError::arbitrum(ERR_NO_BLOCK_NUMBER))?;

            let note_comm = u256_to_scalar(event.note_commitment);
            self.index_note(note_comm, meta).await?;

            if block > most_recent_block {
                most_recent_block = block;
                self.update_latest_block(most_recent_block).await?;
            }
        }

        Ok(())
    }

    /// Index a note
    async fn index_note(
        &self,
        note_comm: NoteCommitment,
        meta: Log,
    ) -> Result<(), FundsManagerError> {
        let tx_hash = meta.transaction_hash.ok_or(FundsManagerError::arbitrum(ERR_NO_TX_HASH))?;
        let maybe_note = self.get_note_from_tx(tx_hash, note_comm).await?;
        let tx = format!("{:#x}", tx_hash);
        let note = match maybe_note {
            Some(note) => note,
            None => {
                info!("not the note receiver, skipping...");
                return Ok(());
            },
        };
        info!("indexing note from tx: {tx}");

        // Check that the note's nullifier has not been spent
        let nullifier = note.nullifier();
        if self
            .darkpool_client
            .check_nullifier_used(nullifier)
            .await
            .map_err(|_| FundsManagerError::db("failed to check nullifier"))?
        {
            info!("note nullifier already spent, skipping");
            return Ok(());
        }

        // Otherwise, index the note
        let fee = NewFee::new_from_note(&note, tx);
        self.insert_fee(fee).await
    }

    /// Get a note from a transaction body
    ///
    /// Checks the note's commitment against the provided commitment, returning
    /// `None` if they do not match
    pub(crate) async fn get_note_from_tx(
        &self,
        tx_hash: TxHash,
        note_comm: NoteCommitment,
    ) -> Result<Option<Note>, FundsManagerError> {
        // Parse the note from the tx then decrypt it
        let cipher = self.get_ciphertext_from_tx(tx_hash).await?;
        Ok(self.decrypt_note(&cipher, note_comm))
    }

    /// Get a note from a transaction body using the given key to decrypt it
    pub(crate) async fn get_note_from_tx_with_key(
        &self,
        tx_hash: TxHash,
        decryption_key: &DecryptionKey,
    ) -> Result<Note, FundsManagerError> {
        // Parse the note from the tx the decrypt
        let cipher = self.get_ciphertext_from_tx(tx_hash).await?;
        Ok(self.decrypt_note_with_key(&cipher, decryption_key))
    }

    /// Get the ciphertext of a note from a tx body
    async fn get_ciphertext_from_tx(
        &self,
        tx_hash: TxHash,
    ) -> Result<ElGamalCiphertext<NOTE_CIPHERTEXT_SIZE>, FundsManagerError> {
        let tx = self
            .darkpool_client
            .provider()
            .get_transaction_by_hash(tx_hash)
            .await
            .map_err(err_str!(FundsManagerError::Arbitrum))?
            .ok_or_else(|| FundsManagerError::arbitrum(ERR_TX_NOT_FOUND))?;

        let calldata: Vec<u8> = tx.inner.input().to_vec();
        let selector: [u8; SELECTOR_LEN] = calldata[..SELECTOR_LEN].try_into().unwrap();
        let encryption = match selector {
            <settleOfflineFeeCall as SolCall>::SELECTOR => {
                parse_note_ciphertext_from_settle_offline_fee(&calldata)?
            },
            sel => {
                return Err(FundsManagerError::arbitrum(format!(
                    "invalid selector when parsing note: {sel:?}"
                )))
            },
        };

        Ok(encryption)
    }

    /// Decrypt a note using the decryption key
    ///
    /// Checks the decryption against the note's expected commitment, returns
    /// `None` if the note does not match for any of the provided key
    fn decrypt_note(
        &self,
        note: &ElGamalCiphertext<NOTE_CIPHERTEXT_SIZE>,
        note_comm: NoteCommitment,
    ) -> Option<Note> {
        if !note.is_valid_ciphertext() {
            warn!("invalid note ciphertext, skipping decryption...");
            return None;
        }

        // The ciphertext stores all note values except the encryption key
        for key in self.decryption_keys.iter() {
            let note = self.decrypt_note_with_key(note, key);
            if note.commitment() == note_comm {
                return Some(note);
            }
        }

        None
    }

    /// Decrypt a note using the given key
    fn decrypt_note_with_key(
        &self,
        note: &ElGamalCiphertext<NOTE_CIPHERTEXT_SIZE>,
        key: &DecryptionKey,
    ) -> Note {
        let cleartext_values: [Scalar; NOTE_CIPHERTEXT_SIZE] = elgamal_decrypt(note, key);
        Note {
            mint: scalar_to_biguint(&cleartext_values[0]),
            amount: scalar_to_u128(&cleartext_values[1]),
            receiver: key.public_key(),
            blinder: cleartext_values[2],
        }
    }
}

// -----------
// | Helpers |
// -----------

/// Parse a note from calldata of a `settleOfflineFee` call
// TODO: Move to renegade_darkpool_client
fn parse_note_ciphertext_from_settle_offline_fee(
    calldata: &[u8],
) -> Result<ElGamalCiphertext<NOTE_CIPHERTEXT_SIZE>, FundsManagerError> {
    let call = settleOfflineFeeCall::abi_decode(calldata).map_err(FundsManagerError::arbitrum)?;

    let statement = deserialize_calldata::<ContractValidOfflineFeeSettlementStatement>(
        &call.valid_offline_fee_settlement_statement,
    )
    .map_err(FundsManagerError::arbitrum)?;

    let ciphertext = statement.note_ciphertext;

    let key_encryption =
        BabyJubJubPoint { x: Scalar::new(ciphertext.0.x), y: Scalar::new(ciphertext.0.y) };

    let symmetric_ciphertext =
        [Scalar::new(ciphertext.1), Scalar::new(ciphertext.2), Scalar::new(ciphertext.3)];

    Ok(ElGamalCiphertext { ephemeral_key: key_encryption, ciphertext: symmetric_ciphertext })
}
