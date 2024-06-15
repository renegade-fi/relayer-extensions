//! Phase one of the sweeper's execution; index all fees since the last consistent block

use alloy_sol_types::SolCall;
use arbitrum_client::abi::settleOfflineFeeCall;
use arbitrum_client::{
    abi::NotePostedFilter, constants::SELECTOR_LEN,
    helpers::parse_note_ciphertext_from_settle_offline_fee,
};
use ethers::contract::LogMeta;
use ethers::middleware::Middleware;
use renegade_circuit_types::elgamal::ElGamalCiphertext;
use renegade_circuit_types::native_helpers::elgamal_decrypt;
use renegade_circuit_types::note::{Note, NOTE_CIPHERTEXT_SIZE};
use renegade_circuit_types::wallet::NoteCommitment;
use renegade_constants::Scalar;
use renegade_crypto::fields::{scalar_to_biguint, scalar_to_u128, u256_to_scalar};
use renegade_util::raw_err_str;

use crate::Indexer;

impl Indexer {
    /// Index all fees since the given block
    pub async fn index_fees(&self, block_number: u64) -> Result<(), String> {
        let filter = self
            .client
            .get_darkpool_client()
            .event::<NotePostedFilter>()
            .from_block(block_number);

        let events = filter
            .query_with_meta()
            .await
            .map_err(raw_err_str!("failed to create note posted stream: {}"))?;

        for (event, meta) in events {
            let note_comm = u256_to_scalar(&event.note_commitment);
            self.index_note(note_comm, meta).await?;
        }

        Ok(())
    }

    /// Index a note
    async fn index_note(&self, note_comm: NoteCommitment, meta: LogMeta) -> Result<(), String> {
        // Parse the note from the tx
        let tx = self
            .client
            .get_darkpool_client()
            .client()
            .get_transaction(meta.transaction_hash)
            .await
            .map_err(raw_err_str!("failed to query tx: {}"))?
            .ok_or_else(|| format!("tx not found: {}", meta.transaction_hash))?;

        let calldata: Vec<u8> = tx.input.to_vec();
        let selector: [u8; 4] = calldata[..SELECTOR_LEN].try_into().unwrap();
        let encryption = match selector {
            <settleOfflineFeeCall as SolCall>::SELECTOR => {
                parse_note_ciphertext_from_settle_offline_fee(&calldata)
                    .map_err(raw_err_str!("failed to parse ciphertext: {}"))?
            }
            sel => return Err(format!("invalid selector when parsing note: {sel:?}")),
        };

        // Decrypt the note and check that the commitment matches the expected value; if not we are not the receiver
        let note = self.decrypt_note(&encryption);
        if note.commitment() != note_comm {
            println!("not receiver, skipping");
            return Ok(());
        }

        // Otherwise, index the note
        // TODO: Write the note info to the DB
        println!("indexed note: {note:?}");

        Ok(())
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
