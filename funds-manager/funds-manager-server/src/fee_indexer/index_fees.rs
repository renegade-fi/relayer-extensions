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
        abi::Darkpool::{settleOfflineFeeCall as ArbSettleOfflineFeeCall, NotePosted},
        contract_types::types::ValidOfflineFeeSettlementStatement as ContractValidOfflineFeeSettlementStatement,
        helpers::deserialize_calldata,
    },
    conversion::u256_to_scalar,
};
use renegade_solidity_abi::IDarkpool::settleOfflineFeeCall as BaseSettleOfflineFeeCall;
use renegade_util::err_str;
use tracing::{info, warn};

use crate::db::models::NewFee;
use crate::error::FundsManagerError;
use crate::Indexer;

/// Block chunk size for querying logs
const BLOCK_CHUNK_SIZE: u64 = 10000;

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
        let latest_block = self.get_latest_block().await?;

        // Get the current block number
        let current_block = self
            .darkpool_client
            .provider()
            .get_block_number()
            .await
            .map_err(FundsManagerError::on_chain)?;

        // Process blocks in chunks
        let mut chunk_start = latest_block;
        while chunk_start <= current_block {
            let chunk_end = std::cmp::min(chunk_start + BLOCK_CHUNK_SIZE - 1, current_block);

            // Index fees from the current chunk
            self.index_fees_from_block_range(chunk_start, chunk_end).await?;

            // Move to next chunk
            chunk_start = chunk_end + 1;
        }

        Ok(())
    }

    /// Index fees from a given block range
    async fn index_fees_from_block_range(
        &self,
        start_block: u64,
        end_block: u64,
    ) -> Result<(), FundsManagerError> {
        info!("indexing fees from blocks {start_block} to {end_block}");

        let filter = self
            .darkpool_client
            .event_filter::<NotePosted>()
            .from_block(start_block)
            .to_block(end_block);

        let events = filter.query().await.map_err(|e| {
            FundsManagerError::on_chain(format!("{ERR_FAILED_TO_CREATE_NOTE_POSTED_STREAM}: {e}"))
        })?;

        let mut most_recent_block = start_block;
        for (event, meta) in events {
            let block =
                meta.block_number.ok_or(FundsManagerError::on_chain(ERR_NO_BLOCK_NUMBER))?;

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
        let tx_hash = meta.transaction_hash.ok_or(FundsManagerError::on_chain(ERR_NO_TX_HASH))?;
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
        let fee = NewFee::new_from_note(&note, tx, self.chain);
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
            .map_err(err_str!(FundsManagerError::on_chain))?
            .ok_or_else(|| FundsManagerError::on_chain(ERR_TX_NOT_FOUND))?;

        let calldata: Vec<u8> = tx.inner.input().to_vec();
        let selector: [u8; SELECTOR_LEN] = calldata[..SELECTOR_LEN].try_into().unwrap();

        let encryption = match selector {
            <ArbSettleOfflineFeeCall as SolCall>::SELECTOR => {
                parse_note_ciphertext_from_arb_settle_offline_fee(&calldata)?
            },
            <BaseSettleOfflineFeeCall as SolCall>::SELECTOR => {
                parse_note_ciphertext_from_base_settle_offline_fee(&calldata)?
            },
            sel => {
                let selector_hex = hex::encode(sel);
                return Err(FundsManagerError::on_chain(format!(
                    "invalid selector when parsing note from tx {tx_hash:#x}: 0x{selector_hex}"
                )));
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

/// Parse a note from calldata of a `settleOfflineFee` call on the Arbitrum
/// darkpool
// TODO: Move to renegade_darkpool_client
fn parse_note_ciphertext_from_arb_settle_offline_fee(
    calldata: &[u8],
) -> Result<ElGamalCiphertext<NOTE_CIPHERTEXT_SIZE>, FundsManagerError> {
    let call =
        ArbSettleOfflineFeeCall::abi_decode(calldata).map_err(FundsManagerError::on_chain)?;

    let statement = deserialize_calldata::<ContractValidOfflineFeeSettlementStatement>(
        &call.valid_offline_fee_settlement_statement,
    )
    .map_err(FundsManagerError::on_chain)?;

    let ciphertext = statement.note_ciphertext;

    let key_encryption =
        BabyJubJubPoint { x: Scalar::new(ciphertext.0.x), y: Scalar::new(ciphertext.0.y) };

    let symmetric_ciphertext =
        [Scalar::new(ciphertext.1), Scalar::new(ciphertext.2), Scalar::new(ciphertext.3)];

    Ok(ElGamalCiphertext { ephemeral_key: key_encryption, ciphertext: symmetric_ciphertext })
}

/// Parse a note from calldata of a `settleOfflineFee` call on the Base
/// darkpool
fn parse_note_ciphertext_from_base_settle_offline_fee(
    calldata: &[u8],
) -> Result<ElGamalCiphertext<NOTE_CIPHERTEXT_SIZE>, FundsManagerError> {
    let call =
        BaseSettleOfflineFeeCall::abi_decode(calldata).map_err(FundsManagerError::on_chain)?;

    let statement = call.statement;
    let note_ciphertext = statement.noteCiphertext;

    let x = u256_to_scalar(note_ciphertext.ephemeralKey.x);
    let y = u256_to_scalar(note_ciphertext.ephemeralKey.y);

    let ephemeral_key = BabyJubJubPoint { x, y };
    let ciphertext = [
        u256_to_scalar(note_ciphertext.ciphertext[0]),
        u256_to_scalar(note_ciphertext.ciphertext[1]),
        u256_to_scalar(note_ciphertext.ciphertext[2]),
    ];

    Ok(ElGamalCiphertext { ephemeral_key, ciphertext })
}
