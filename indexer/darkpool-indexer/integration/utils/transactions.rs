//! Utilities for sending blockchain transactions

use std::time::Duration;

use alloy::{
    contract::{CallBuilder, CallDecoder},
    network::Ethereum,
    providers::{DynProvider, Provider},
    rpc::types::TransactionReceipt,
    sol_types::SolEvent,
};
use eyre::Result;
use renegade_constants::Scalar;
use renegade_crypto::fields::u256_to_scalar;
use renegade_solidity_abi::v2::IDarkpoolV2::MerkleInsertion;
use test_helpers::assert_eq_result;

// ---------
// | Types |
// ---------

/// The call builder type for the tests
pub type TestCallBuilder<'a, C> = CallBuilder<&'a DynProvider, C, Ethereum>;

// -----------
// | Helpers |
// -----------

/// Wait for a transaction receipt and ensure it was successful
pub async fn wait_for_tx_success<C: CallDecoder>(
    tx: TestCallBuilder<'_, C>,
) -> Result<TransactionReceipt> {
    let receipt = send_tx(tx).await?;
    assert_eq_result!(receipt.status(), true)?;
    Ok(receipt)
}

/// Send a transaction and wait for it to succeed or fail
pub async fn send_tx<C: CallDecoder>(tx: TestCallBuilder<'_, C>) -> Result<TransactionReceipt> {
    let pending_tx = tx.send().await?;

    let tx_hash = *pending_tx.tx_hash();

    // Retry fetching the receipt up to 10 times
    // The current version of alloy has issues watching the pending transaction
    // directly, so we patch this here
    let mut remaining_attempts = 10;
    let provider = tx.provider;
    while remaining_attempts > 0 {
        match provider.get_transaction_receipt(tx_hash).await? {
            Some(receipt) => return Ok(receipt),
            None => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                remaining_attempts -= 1;
            },
        }
    }

    eyre::bail!("no tx receipt found after retries");
}

/// Get the first value inserted as a Merkle tree leaf in the given transaction
pub async fn get_first_merkle_insertion(receipt: &TransactionReceipt) -> Result<Scalar> {
    receipt
        .logs()
        .iter()
        .find_map(|log| {
            MerkleInsertion::decode_log(&log.inner)
                .ok()
                .map(|insertion| insertion.value)
                .map(|value_u256| u256_to_scalar(&value_u256))
        })
        .ok_or(eyre::eyre!("No Merkle insertion found in receipt"))
}
