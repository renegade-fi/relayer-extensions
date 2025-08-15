//! Handlers for quoter endpoints

use std::sync::Arc;

use funds_manager_api::quoters::{
    DepositAddressResponse, WithdrawFundsRequest, WithdrawToHyperliquidRequest,
};
use renegade_common::types::chain::Chain;
use tracing::warn;
use warp::reply::Json;

use crate::{
    cli::Environment, custody_client::DepositWithdrawSource, error::ApiError, server::Server,
};

// -------------
// | Constants |
// -------------

/// The maximum value of a quoter withdrawal that can be processed in a single
/// request
const MAX_WITHDRAWAL_VALUE: f64 = 50_000.; // USD
/// The minimum amount of USDC that can be deposited into Hyperliquid
const MIN_HYPERLIQUID_DEPOSIT_AMOUNT: f64 = 5.0; // USDC

/// Handler for withdrawing funds from custody
pub(crate) async fn quoter_withdraw_handler(
    chain: Chain,
    withdraw_request: WithdrawFundsRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // Get the price of the token
    let maybe_price = server.price_reporter.get_price(&withdraw_request.mint, chain).await;

    match maybe_price {
        Ok(price) => {
            // If a price was found, check that the withdrawal value is less than the
            // allowable maximum. If no price was found, we do not block the
            // withdrawal.

            let value = withdraw_request.amount * price;
            if value > MAX_WITHDRAWAL_VALUE {
                return Err(warp::reject::custom(ApiError::BadRequest(format!(
                    "Requested withdrawal of ${} of {} exceeds maximum allowed withdrawal of ${}",
                    value, withdraw_request.mint, MAX_WITHDRAWAL_VALUE
                ))));
            }
        },
        Err(e) => {
            warn!("Error getting price for {}, allowing withdrawal: {e}", withdraw_request.mint);
        },
    }

    // Withdraw the funds
    let custody_client = server.get_custody_client(&chain)?;

    custody_client
        .withdraw_from_hot_wallet(
            DepositWithdrawSource::Quoter,
            &withdraw_request.address,
            &withdraw_request.mint,
            withdraw_request.amount,
        )
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    Ok(warp::reply::json(&"Withdrawal complete"))
}

/// Handler for retrieving the address to deposit custody funds to
pub(crate) async fn get_deposit_address_handler(
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;
    let address = custody_client
        .get_deposit_address(DepositWithdrawSource::Quoter)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = DepositAddressResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for withdrawing USDC to Hyperliquid
pub(crate) async fn withdraw_to_hyperliquid_handler(
    req: WithdrawToHyperliquidRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // TODO: Separate out chain-agnostic hedging client from custody client
    let chain = match server.environment {
        Environment::Mainnet => Chain::ArbitrumOne,
        Environment::Testnet => Chain::ArbitrumSepolia,
    };

    if req.amount < MIN_HYPERLIQUID_DEPOSIT_AMOUNT {
        return Err(warp::reject::custom(ApiError::BadRequest(format!(
            "Requested amount {} USDC is less than the minimum allowed deposit of {} USDC",
            req.amount, MIN_HYPERLIQUID_DEPOSIT_AMOUNT
        ))));
    }

    let custody_client = server.get_custody_client(&chain)?;
    custody_client.withdraw_to_hyperliquid(req.amount).await?;

    Ok(warp::reply::json(&"Withdrawal to Hyperliquid complete"))
}
