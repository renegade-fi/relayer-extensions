//! Integration testing utilities for generating test data

use alloy::primitives::U256;
use eyre::Result;
use rand::{Rng, thread_rng};
use renegade_circuit_types::{
    fixed_point::FixedPoint, intent::Intent, max_amount,
    settlement_obligation::SettlementObligation,
};
use renegade_circuits::test_helpers::{
    BOUNDED_MAX_AMT, compute_implied_price, compute_min_amount_out, random_price,
};
use renegade_solidity_abi::v2::IDarkpoolV2::Deposit;

use crate::test_args::TestArgs;

/// Generate a random circuit-compatible amount as a U256.
///
/// The amount will be of size at most 2 ** AMOUNT_BITS
pub fn random_amount_u256() -> U256 {
    let amount_u128 = thread_rng().gen_range(0..=BOUNDED_MAX_AMT);
    U256::from(amount_u128)
}

/// Generate a deposit for the first test account w/ a random amount
pub fn random_deposit(args: &TestArgs) -> Result<Deposit> {
    Ok(Deposit {
        from: args.party0_address(),
        token: args.base_token_address()?,
        amount: random_amount_u256(),
    })
}

/// The settlement relayer fee to use for testing
pub fn settlement_relayer_fee() -> FixedPoint {
    FixedPoint::from_f64_round_down(0.0001) // 1bp
}

/// Create two matching intents and obligations
///
/// Party 0 sells the base; party 1 sells the quote
pub fn create_intents_and_obligations(
    args: &TestArgs,
) -> Result<(Intent, Intent, SettlementObligation, SettlementObligation)> {
    // Construct a random intent for the first party
    let mut rng = thread_rng();
    let amount_in = rng.gen_range(0..=BOUNDED_MAX_AMT);
    let min_price = random_price();
    let intent0 = Intent {
        in_token: args.base_token_address()?,
        out_token: args.quote_token_address()?,
        owner: args.party0_address(),
        min_price,
        amount_in,
    };

    let counterparty = args.party1_address();

    // Determine the trade parameters
    let party0_amt_in = rng.gen_range(0..intent0.amount_in);
    let min_amt_out = compute_min_amount_out(&intent0, party0_amt_in);
    let party0_amt_out = rng.gen_range(min_amt_out..=max_amount());

    // Build two compatible obligations
    let obligation0 = SettlementObligation {
        input_token: intent0.in_token,
        output_token: intent0.out_token,
        amount_in: party0_amt_in,
        amount_out: party0_amt_out,
    };
    let obligation1 = SettlementObligation {
        input_token: intent0.out_token,
        output_token: intent0.in_token,
        amount_in: party0_amt_out,
        amount_out: party0_amt_in,
    };

    // Create a compatible intent for the counterparty
    let trade_price = compute_implied_price(obligation1.amount_out, obligation1.amount_in);

    let min_price = trade_price.floor_div(&FixedPoint::from(2_u128));
    let amount_in = rng.gen_range(party0_amt_out..=max_amount());
    let intent1 = Intent {
        in_token: intent0.out_token,
        out_token: intent0.in_token,
        owner: counterparty,
        min_price,
        amount_in,
    };

    Ok((intent0, intent1, obligation0, obligation1))
}

/// Split an obligation in two
///
/// Returns the two splits of the obligation
pub fn split_obligation(
    obligation: &SettlementObligation,
) -> (SettlementObligation, SettlementObligation) {
    let mut obligation0 = obligation.clone();
    let mut obligation1 = obligation.clone();
    obligation0.amount_in /= 2;
    obligation0.amount_out /= 2;
    obligation1.amount_in /= 2;
    obligation1.amount_out /= 2;

    (obligation0, obligation1)
}
