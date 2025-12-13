//! Common assertion helpers for integration tests

use alloy::providers::DynProvider;
use eyre::Result;
use renegade_circuit_types::{
    state_wrapper::StateWrapper,
    traits::{CircuitBaseType, SecretShareBaseType},
};
use renegade_solidity_abi::v2::IDarkpoolV2::IDarkpoolV2Instance;
use test_helpers::assert_eq_result;

use crate::utils::merkle::find_commitment;

/// Assert that a state object is committed to the onchain Merkle tree
pub async fn assert_state_object_committed<T>(
    state_object: &StateWrapper<T>,
    darkpool: &IDarkpoolV2Instance<DynProvider>,
) -> Result<()>
where
    T: SecretShareBaseType + CircuitBaseType,
    T::ShareType: CircuitBaseType,
{
    let commitment = state_object.compute_commitment();
    let commitment_found = find_commitment(commitment, darkpool).await.is_ok();

    assert_eq_result!(commitment_found, true)
}
