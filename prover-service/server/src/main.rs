//! A service for generating proofs of Renegade circuits

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::unused_async)]
// Increase compiler type recursion limit to support deeply nested `warp` filter types
#![recursion_limit = "256"]

use std::net::SocketAddr;

use clap::Parser;
use http::StatusCode;
use renegade_circuit_types::traits::{SingleProverCircuit, setup_preprocessed_keys};
use renegade_circuits_core::zk_circuits::{
    // Fee proofs
    fees::{
        valid_note_redemption::SizedValidNoteRedemption,
        valid_private_protocol_fee_payment::SizedValidPrivateProtocolFeePayment,
        valid_private_relayer_fee_payment::SizedValidPrivateRelayerFeePayment,
        valid_public_protocol_fee_payment::SizedValidPublicProtocolFeePayment,
        valid_public_relayer_fee_payment::SizedValidPublicRelayerFeePayment,
    },
    // Settlement proofs
    settlement::{
        intent_and_balance_bounded_settlement::IntentAndBalanceBoundedSettlementCircuit,
        intent_and_balance_private_settlement::IntentAndBalancePrivateSettlementCircuit,
        intent_and_balance_public_settlement::IntentAndBalancePublicSettlementCircuit,
        intent_only_bounded_settlement::IntentOnlyBoundedSettlementCircuit,
        intent_only_public_settlement::IntentOnlyPublicSettlementCircuit,
    },
    // Update proofs
    valid_balance_create::ValidBalanceCreate,
    valid_deposit::SizedValidDeposit,
    valid_order_cancellation::SizedValidOrderCancellationCircuit,
    valid_withdrawal::SizedValidWithdrawal,
    // Validity proofs
    validity_proofs::{
        intent_and_balance::SizedIntentAndBalanceValidityCircuit,
        intent_and_balance_first_fill::SizedIntentAndBalanceFirstFillValidityCircuit,
        intent_only::SizedIntentOnlyValidityCircuit,
        intent_only_first_fill::IntentOnlyFirstFillValidityCircuit,
        new_output_balance::SizedNewOutputBalanceValidityCircuit,
        output_balance::SizedOutputBalanceValidityCircuit,
    },
};
use tracing::{error, info};
use warp::{Filter, reject::Rejection, reply::Reply};

use crate::{
    cli::Cli,
    error::ProverServiceError,
    middleware::{basic_auth, handle_rejection, propagate_span, with_tracing},
    prover::{
        handle_intent_and_balance_bounded_settlement,
        handle_intent_and_balance_first_fill_validity,
        handle_intent_and_balance_private_settlement, handle_intent_and_balance_public_settlement,
        handle_intent_and_balance_validity, handle_intent_only_bounded_settlement,
        handle_intent_only_first_fill_validity, handle_intent_only_public_settlement,
        handle_intent_only_validity, handle_new_output_balance_validity,
        handle_output_balance_validity, handle_valid_balance_create, handle_valid_deposit,
        handle_valid_note_redemption, handle_valid_order_cancellation,
        handle_valid_private_protocol_fee_payment, handle_valid_private_relayer_fee_payment,
        handle_valid_public_protocol_fee_payment, handle_valid_public_relayer_fee_payment,
        handle_valid_withdrawal,
    },
};

mod cli;
mod error;
mod middleware;
mod prover;

/// The runtime stack size to use for the server
const RUNTIME_STACK_SIZE: usize = 50 * 1024 * 1024; // 50MB

/// Entrypoint
fn main() {
    // Create a custom tokio runtime with 50MB stack size
    // The warp filters sometimes overflow the stack in debug mode; so we manually
    // setup the stack
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(RUNTIME_STACK_SIZE)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    runtime.block_on(async_main());
}

/// Async main function
async fn async_main() {
    let cli = Cli::parse();
    cli.configure_telemetry().expect("failed to setup telemetry");

    // Setup the circuits
    tokio::task::spawn_blocking(|| {
        if let Err(e) = preprocess_circuits() {
            error!("failed to setup circuits: {e}");
        }
    })
    .await
    .expect("failed to setup circuits");

    // Run the server
    let routes = setup_routes(cli.auth_password);
    let listen_addr: SocketAddr = ([0, 0, 0, 0], cli.port).into();
    info!("listening on {}", listen_addr);
    warp::serve(routes).bind(listen_addr).await;
}

// --- Setup --- //

/// Initialize the proving key/verification key & circuit layout caches for
/// all of the circuits
fn preprocess_circuits() -> Result<(), ProverServiceError> {
    // Set up the proving & verification keys for all of the circuits

    // Update proofs
    setup_preprocessed_keys::<ValidBalanceCreate>();
    setup_preprocessed_keys::<SizedValidDeposit>();
    setup_preprocessed_keys::<SizedValidOrderCancellationCircuit>();
    setup_preprocessed_keys::<SizedValidWithdrawal>();

    // Validity proofs
    setup_preprocessed_keys::<SizedIntentAndBalanceValidityCircuit>();
    setup_preprocessed_keys::<SizedIntentAndBalanceFirstFillValidityCircuit>();
    setup_preprocessed_keys::<SizedIntentOnlyValidityCircuit>();
    setup_preprocessed_keys::<IntentOnlyFirstFillValidityCircuit>();
    setup_preprocessed_keys::<SizedNewOutputBalanceValidityCircuit>();
    setup_preprocessed_keys::<SizedOutputBalanceValidityCircuit>();

    // Settlement proofs
    setup_preprocessed_keys::<IntentAndBalanceBoundedSettlementCircuit>();
    setup_preprocessed_keys::<IntentAndBalancePrivateSettlementCircuit>();
    setup_preprocessed_keys::<IntentAndBalancePublicSettlementCircuit>();
    setup_preprocessed_keys::<IntentOnlyBoundedSettlementCircuit>();
    setup_preprocessed_keys::<IntentOnlyPublicSettlementCircuit>();

    // Fee proofs
    setup_preprocessed_keys::<SizedValidNoteRedemption>();
    setup_preprocessed_keys::<SizedValidPrivateProtocolFeePayment>();
    setup_preprocessed_keys::<SizedValidPrivateRelayerFeePayment>();
    setup_preprocessed_keys::<SizedValidPublicProtocolFeePayment>();
    setup_preprocessed_keys::<SizedValidPublicRelayerFeePayment>();

    // Set up layouts for all of the circuits

    // Update proofs
    ValidBalanceCreate::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedValidDeposit::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedValidOrderCancellationCircuit::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedValidWithdrawal::get_circuit_layout().map_err(ProverServiceError::setup)?;

    // Validity proofs
    SizedIntentAndBalanceValidityCircuit::get_circuit_layout()
        .map_err(ProverServiceError::setup)?;

    SizedIntentAndBalanceFirstFillValidityCircuit::get_circuit_layout()
        .map_err(ProverServiceError::setup)?;

    SizedIntentOnlyValidityCircuit::get_circuit_layout().map_err(ProverServiceError::setup)?;
    IntentOnlyFirstFillValidityCircuit::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedNewOutputBalanceValidityCircuit::get_circuit_layout()
        .map_err(ProverServiceError::setup)?;

    SizedOutputBalanceValidityCircuit::get_circuit_layout().map_err(ProverServiceError::setup)?;

    // Settlement proofs
    IntentAndBalanceBoundedSettlementCircuit::get_circuit_layout()
        .map_err(ProverServiceError::setup)?;

    IntentAndBalancePrivateSettlementCircuit::get_circuit_layout()
        .map_err(ProverServiceError::setup)?;

    IntentAndBalancePublicSettlementCircuit::get_circuit_layout()
        .map_err(ProverServiceError::setup)?;

    IntentOnlyBoundedSettlementCircuit::get_circuit_layout().map_err(ProverServiceError::setup)?;
    IntentOnlyPublicSettlementCircuit::get_circuit_layout().map_err(ProverServiceError::setup)?;

    // Fee proofs
    SizedValidNoteRedemption::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedValidPrivateProtocolFeePayment::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedValidPrivateRelayerFeePayment::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedValidPublicProtocolFeePayment::get_circuit_layout().map_err(ProverServiceError::setup)?;
    SizedValidPublicRelayerFeePayment::get_circuit_layout().map_err(ProverServiceError::setup)?;

    Ok(())
}

// --- Routes --- //

/// Setup the HTTP routes
fn setup_routes(
    auth_pwd: String,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    // Ping route
    let ping = warp::path("ping")
        .and(warp::get())
        .map(|| warp::reply::with_status("PONG", StatusCode::OK));

    // --- Update Proofs --- //

    let valid_balance_create = warp::path("prove-valid-balance-create")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_balance_create);

    let valid_deposit = warp::path("prove-valid-deposit")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_deposit);

    let valid_order_cancellation = warp::path("prove-valid-order-cancellation")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_order_cancellation);

    let valid_withdrawal = warp::path("prove-valid-withdrawal")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_withdrawal);

    // --- Validity Proofs --- //

    let intent_and_balance_validity = warp::path("prove-intent-and-balance-validity")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_intent_and_balance_validity);

    let intent_and_balance_first_fill_validity =
        warp::path("prove-intent-and-balance-first-fill-validity")
            .and(warp::post())
            .and(propagate_span())
            .and(basic_auth(auth_pwd.clone()))
            .and(warp::body::json())
            .and_then(handle_intent_and_balance_first_fill_validity);

    let intent_only_validity = warp::path("prove-intent-only-validity")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_intent_only_validity);

    let intent_only_first_fill_validity = warp::path("prove-intent-only-first-fill-validity")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_intent_only_first_fill_validity);

    let new_output_balance_validity = warp::path("prove-new-output-balance-validity")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_new_output_balance_validity);

    let output_balance_validity = warp::path("prove-output-balance-validity")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_output_balance_validity);

    // --- Settlement Proofs --- //

    let intent_and_balance_bounded_settlement =
        warp::path("prove-intent-and-balance-bounded-settlement")
            .and(warp::post())
            .and(propagate_span())
            .and(basic_auth(auth_pwd.clone()))
            .and(warp::body::json())
            .and_then(handle_intent_and_balance_bounded_settlement);

    let intent_and_balance_private_settlement =
        warp::path("prove-intent-and-balance-private-settlement")
            .and(warp::post())
            .and(propagate_span())
            .and(basic_auth(auth_pwd.clone()))
            .and(warp::body::json())
            .and_then(handle_intent_and_balance_private_settlement);

    let intent_and_balance_public_settlement =
        warp::path("prove-intent-and-balance-public-settlement")
            .and(warp::post())
            .and(propagate_span())
            .and(basic_auth(auth_pwd.clone()))
            .and(warp::body::json())
            .and_then(handle_intent_and_balance_public_settlement);

    let intent_only_bounded_settlement = warp::path("prove-intent-only-bounded-settlement")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_intent_only_bounded_settlement);

    let intent_only_public_settlement = warp::path("prove-intent-only-public-settlement")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_intent_only_public_settlement);

    // --- Fee Proofs --- //

    let valid_note_redemption = warp::path("prove-valid-note-redemption")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_note_redemption);

    let valid_private_protocol_fee_payment = warp::path("prove-valid-private-protocol-fee-payment")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_private_protocol_fee_payment);

    let valid_private_relayer_fee_payment = warp::path("prove-valid-private-relayer-fee-payment")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_private_relayer_fee_payment);

    let valid_public_protocol_fee_payment = warp::path("prove-valid-public-protocol-fee-payment")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd.clone()))
        .and(warp::body::json())
        .and_then(handle_valid_public_protocol_fee_payment);

    let valid_public_relayer_fee_payment = warp::path("prove-valid-public-relayer-fee-payment")
        .and(warp::post())
        .and(propagate_span())
        .and(basic_auth(auth_pwd))
        .and(warp::body::json())
        .and_then(handle_valid_public_relayer_fee_payment);

    // Combine all routes
    ping
        // Update proofs
        .or(valid_balance_create)
        .or(valid_deposit)
        .or(valid_order_cancellation)
        .or(valid_withdrawal)
        // Validity proofs
        .or(intent_and_balance_validity)
        .or(intent_and_balance_first_fill_validity)
        .or(intent_only_validity)
        .or(intent_only_first_fill_validity)
        .or(new_output_balance_validity)
        .or(output_balance_validity)
        // Settlement proofs
        .or(intent_and_balance_bounded_settlement)
        .or(intent_and_balance_private_settlement)
        .or(intent_and_balance_public_settlement)
        .or(intent_only_bounded_settlement)
        .or(intent_only_public_settlement)
        // Fee proofs
        .or(valid_note_redemption)
        .or(valid_private_protocol_fee_payment)
        .or(valid_private_relayer_fee_payment)
        .or(valid_public_protocol_fee_payment)
        .or(valid_public_relayer_fee_payment)
        .with(with_tracing())
        .recover(handle_rejection)
}

// --- Middleware --- //
