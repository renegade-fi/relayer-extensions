//! Tests the indexing of a `depositNewBalance` contract call

use eyre::Result;
use test_helpers::integration_test_async;

use crate::{
    test_args::TestArgs,
    utils::balance::{deposit_new_balance, random_deposit},
};

/// Test the indexing of a `depositNewBalance` call
async fn test_deposit_new_balance(mut args: TestArgs) -> Result<()> {
    let deposit = random_deposit(&args)?;
    let (_receipt, _balance) = deposit_new_balance(&mut args, &deposit).await?;

    // TODO: Assert valid indexing
    Ok(())
}
integration_test_async!(test_deposit_new_balance);
