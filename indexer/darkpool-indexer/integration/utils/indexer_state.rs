//! Integration testing utilities for fetching indexer state

use darkpool_indexer::api::http::handlers::get_all_active_user_state_objects;
use darkpool_indexer_api::types::http::{ApiPublicIntent, ApiStateObject};
use eyre::Result;
use renegade_circuit_types::{balance::DarkpoolStateBalance, intent::DarkpoolStateIntent};

use crate::test_args::TestArgs;

/// Get the first balance state object for the first test account
pub async fn get_party0_first_balance(args: &TestArgs) -> Result<DarkpoolStateBalance> {
    let state_objects =
        get_all_active_user_state_objects(args.party0_account_id(), args.db_client()).await?;

    state_objects
        .into_iter()
        .find_map(|state_object| match state_object {
            ApiStateObject::Balance(balance) => Some(balance.balance),
            _ => None,
        })
        .ok_or(eyre::eyre!("Balance not found"))
}

/// Get the first public intent state object for the first test account
pub async fn get_party0_first_public_intent(args: &TestArgs) -> Result<ApiPublicIntent> {
    let state_objects =
        get_all_active_user_state_objects(args.party0_account_id(), args.db_client()).await?;

    state_objects
        .into_iter()
        .find_map(|state_object| match state_object {
            ApiStateObject::PublicIntent(public_intent) => Some(public_intent),
            _ => None,
        })
        .ok_or(eyre::eyre!("Public intent not found"))
}

/// Get the first intent state object for the first test account
pub async fn get_party0_first_intent(args: &TestArgs) -> Result<DarkpoolStateIntent> {
    let state_objects =
        get_all_active_user_state_objects(args.party0_account_id(), args.db_client()).await?;

    state_objects
        .into_iter()
        .find_map(|state_object| match state_object {
            ApiStateObject::Intent(intent) => Some(intent.intent),
            _ => None,
        })
        .ok_or(eyre::eyre!("Intent not found"))
}
