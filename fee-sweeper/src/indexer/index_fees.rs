//! Phase one of the sweeper's execution; index all fees since the last consistent block

use alloy_sol_types::SolCall;
use arbitrum_client::abi::settleOfflineFeeCall;
use arbitrum_client::{
    abi::NotePostedFilter, constants::SELECTOR_LEN,
    helpers::parse_note_ciphertext_from_settle_offline_fee,
};
use ethers::contract::LogMeta;
use ethers::middleware::Middleware;
use ethers::types::TxHash;
use renegade_circuit_types::elgamal::ElGamalCiphertext;
use renegade_circuit_types::native_helpers::elgamal_decrypt;
use renegade_circuit_types::note::{Note, NOTE_CIPHERTEXT_SIZE};
use renegade_circuit_types::wallet::NoteCommitment;
use renegade_constants::Scalar;
use renegade_crypto::fields::{scalar_to_biguint, scalar_to_u128, u256_to_scalar};
use renegade_util::raw_err_str;
use tracing::info;

use crate::db::models::NewFee;
use crate::Indexer;

impl Indexer {
    /// Index all fees since the given block
    pub async fn index_fees(&mut self) -> Result<(), String> {
        let block_number = self.get_latest_block()?;
        info!("indexing fees from block {block_number}");

        let filter = self
            .arbitrum_client
            .get_darkpool_client()
            .event::<NotePostedFilter>()
            .from_block(block_number);

        let events = filter
            .query_with_meta()
            .await
            .map_err(raw_err_str!("failed to create note posted stream: {}"))?;

        let mut most_recent_block = block_number;
        for (event, meta) in events {
            let block = meta.block_number.as_u64();
            let note_comm = u256_to_scalar(&event.note_commitment);
            self.index_note(note_comm, meta).await?;

            if block > most_recent_block {
                most_recent_block = block;
                self.update_latest_block(most_recent_block)?;
            }
        }

        Ok(())
    }

    /// Index a note
    async fn index_note(&mut self, note_comm: NoteCommitment, meta: LogMeta) -> Result<(), String> {
        let note = self.get_note_from_tx(meta.transaction_hash).await?;
        let tx = format!("{:#x}", meta.transaction_hash);
        if note.commitment() != note_comm {
            info!("not receiver, skipping");
            return Ok(());
        } else {
            info!("indexing note from tx: {tx}");
        }

        // Check that the note's nullifier has not been spent
        let nullifier = note.nullifier();
        if self
            .arbitrum_client
            .check_nullifier_used(nullifier)
            .await
            .map_err(raw_err_str!("failed to check nullifier: {}"))?
        {
            info!("note nullifier already spent, skipping");
            return Ok(());
        }

        // Otherwise, index the note
        let fee = NewFee::new_from_note(&note, tx);
        self.insert_fee(fee)
    }

    /// Get a note from a transaction body
    pub(crate) async fn get_note_from_tx(&self, tx_hash: TxHash) -> Result<Note, String> {
        // Parse the note from the tx
        let tx = self
            .arbitrum_client
            .get_darkpool_client()
            .client()
            .get_transaction(tx_hash)
            .await
            .map_err(raw_err_str!("failed to query tx: {}"))?
            .ok_or_else(|| format!("tx not found: {}", tx_hash))?;

        let calldata: Vec<u8> = tx.input.to_vec();
        let selector: [u8; 4] = calldata[..SELECTOR_LEN].try_into().unwrap();
        let encryption = match selector {
            <settleOfflineFeeCall as SolCall>::SELECTOR => {
                parse_note_ciphertext_from_settle_offline_fee(&calldata)
                    .map_err(raw_err_str!("failed to parse ciphertext: {}"))?
            },
            sel => return Err(format!("invalid selector when parsing note: {sel:?}")),
        };

        // Decrypt the note
        let note = self.decrypt_note(&encryption);
        Ok(note)
    }

    /// Decrypt a note using the decryption key
    fn decrypt_note(&self, note: &ElGamalCiphertext<NOTE_CIPHERTEXT_SIZE>) -> Note {
        // The ciphertext stores all note values except the encryption key
        let cleartext_values: [Scalar; NOTE_CIPHERTEXT_SIZE] =
            elgamal_decrypt(note, &self.decryption_key);

        Note {
            mint: scalar_to_biguint(&cleartext_values[0]),
            amount: scalar_to_u128(&cleartext_values[1]),
            receiver: self.decryption_key.public_key(),
            blinder: cleartext_values[2],
        }
    }
}
