//! Utilities for sending blockchain transactions

use alloy::{
    contract::{CallBuilder, CallDecoder},
    network::Ethereum,
    providers::{DynProvider, Provider},
    rpc::types::TransactionReceipt,
};
use eyre::Result;
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
    let receipt = tx
        .provider
        .get_transaction_receipt(tx_hash)
        .await?
        .ok_or(eyre::eyre!("Transaction receipt not found for tx {tx_hash:#x}"))?;

    Ok(receipt)
}
