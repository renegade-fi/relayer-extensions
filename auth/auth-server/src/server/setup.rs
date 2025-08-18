//! Helpers for setting up the server

use super::Server;
use super::db::{create_db_pool, create_redis_client};
use super::gas_estimation::gas_cost_sampler::GasCostSampler;
use super::rate_limiter::AuthServerRateLimiter;

use std::{iter, sync::Arc, time::Duration};

use crate::bundle_store::BundleStore;
use crate::chain_events::listener::{OnChainEventListener, OnChainEventListenerConfig};
use crate::server::caching::ServerCache;
use crate::telemetry::configure_telemtry_from_args;
use crate::{
    Cli,
    error::AuthServerError,
    telemetry::{quote_comparison::handler::QuoteComparisonHandler, sources::QuoteSource},
};
use aes_gcm::{Aes128Gcm, KeyInit};
use alloy::hex;
use alloy::signers::k256::ecdsa::SigningKey;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::Address;
use base64::{Engine, engine::general_purpose};
use price_reporter_client::PriceReporterClient;
use renegade_common::types::chain::Chain;
use renegade_common::types::{
    hmac::HmacKey,
    token::{Token, get_all_tokens},
};
use renegade_config::setup_token_remaps;
use renegade_constants::NATIVE_ASSET_ADDRESS;
use renegade_darkpool_client::DarkpoolClient;
use renegade_darkpool_client::client::DarkpoolClientConfig;
use renegade_system_clock::SystemClock;
use renegade_util::on_chain::{PROTOCOL_FEE, set_external_match_fee};
use reqwest::Client;

/// The interval at which we poll filter updates
const DEFAULT_BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(100);

impl Server {
    /// Create a new server instance
    pub async fn setup(args: Cli, system_clock: &SystemClock) -> Result<Self, AuthServerError> {
        configure_telemtry_from_args(&args)?;
        setup_token_mapping(&args).await?;

        // Create the darkpool client
        let darkpool_client = create_darkpool_client(
            args.darkpool_address.clone(),
            args.chain_id,
            args.rpc_url.clone(),
        )
        .expect("failed to create darkpool client");

        // Set the external match fees & protocol fee
        set_external_match_fees(&darkpool_client).await?;

        // Setup the DB connection pool and the Redis client
        let db_pool = create_db_pool(&args.database_url).await?;
        let redis_client = create_redis_client(&args.redis_url).await?;
        let (encryption_key, management_key, relayer_admin_key, gas_sponsor_auth_key) =
            parse_auth_server_keys(&args)?;

        let rate_limiter = AuthServerRateLimiter::new(
            args.quote_rate_limit,
            args.bundle_rate_limit,
            args.max_gas_sponsorship_value,
            &args.redis_url,
            &args.execution_cost_redis_url,
        )
        .await?;

        let price_reporter_client = PriceReporterClient::new(
            args.price_reporter_url.clone(),
            true, // exit_on_stale
        )?;

        // Setup quote metrics
        let quote_metrics = maybe_setup_quote_metrics(
            &args,
            darkpool_client.clone(),
            price_reporter_client.clone(),
        );

        let gas_sponsor_address = parse_gas_sponsor_address(&args)?;
        let gas_cost_sampler = Arc::new(
            GasCostSampler::new(
                darkpool_client.provider().clone(),
                gas_sponsor_address,
                system_clock,
            )
            .await?,
        );

        // Create the shared in-memory bundle store
        let bundle_store = BundleStore::new();

        // Start the on-chain event listener
        let chain_listener_config = OnChainEventListenerConfig {
            websocket_addr: args.eth_websocket_addr.clone(),
            darkpool_client: darkpool_client.clone(),
        };
        let mut chain_listener = OnChainEventListener::new(
            chain_listener_config,
            bundle_store.clone(),
            rate_limiter.clone(),
            price_reporter_client.clone(),
            gas_cost_sampler.clone(),
            args.chain_id,
            gas_sponsor_address,
        )
        .expect("failed to build on-chain event listener");
        chain_listener.start().expect("failed to start on-chain event listener");
        chain_listener.watch();

        Ok(Self {
            chain: args.chain_id,
            db_pool,
            redis_client,
            relayer_url: args.relayer_url,
            relayer_admin_key,
            management_key,
            encryption_key,
            cache: ServerCache::new(),
            client: Client::new(),
            rate_limiter,
            quote_metrics,
            metrics_sampling_rate: args
                .metrics_sampling_rate
                .unwrap_or(1.0 /* default no sampling */),
            gas_sponsor_address,
            gas_sponsor_auth_key,
            price_reporter_client,
            gas_cost_sampler,
            min_sponsored_order_quote_amount: args.min_sponsored_order_quote_amount,
            bundle_store,
        })
    }
}

// -----------------
// | Setup Helpers |
// -----------------

/// Setup the token mapping
async fn setup_token_mapping(args: &Cli) -> Result<(), AuthServerError> {
    let chain_id = args.chain_id;
    let token_remap_file = args.token_remap_file.clone();
    tokio::task::spawn_blocking(move || setup_token_remaps(token_remap_file, chain_id))
        .await
        .unwrap()
        .map_err(AuthServerError::setup)
}

/// Set the external match fees & protocol fee
async fn set_external_match_fees(darkpool_client: &DarkpoolClient) -> Result<(), AuthServerError> {
    let protocol_fee = darkpool_client.get_protocol_fee().await.map_err(AuthServerError::setup)?;

    PROTOCOL_FEE
        .set(protocol_fee)
        .map_err(|_| AuthServerError::setup("Failed to set protocol fee"))?;

    let tokens: Vec<Token> = get_all_tokens()
        .into_iter()
        .chain(iter::once(Token::from_addr(NATIVE_ASSET_ADDRESS)))
        .collect();

    for token in tokens {
        // Fetch the fee override from the contract
        let addr = token.get_alloy_address();
        let fee =
            darkpool_client.get_external_match_fee(addr).await.map_err(AuthServerError::setup)?;

        // Write the fee into the mapping
        let addr_bigint = token.get_addr_biguint();
        set_external_match_fee(&addr_bigint, fee);
    }

    Ok(())
}

/// Parse the encryption key, management key, relayer admin key, and gas sponsor
/// auth key
fn parse_auth_server_keys(
    args: &Cli,
) -> Result<(Aes128Gcm, HmacKey, HmacKey, SigningKey), AuthServerError> {
    let encryption_key_bytes =
        general_purpose::STANDARD.decode(&args.encryption_key).map_err(AuthServerError::setup)?;

    let encryption_key =
        Aes128Gcm::new_from_slice(&encryption_key_bytes).map_err(AuthServerError::setup)?;

    let management_key =
        HmacKey::from_base64_string(&args.management_key).map_err(AuthServerError::setup)?;

    let relayer_admin_key =
        HmacKey::from_base64_string(&args.relayer_admin_key).map_err(AuthServerError::setup)?;

    let gas_sponsor_auth_key_bytes =
        hex::decode(&args.gas_sponsor_auth_key).map_err(AuthServerError::setup)?;

    let gas_sponsor_auth_key =
        SigningKey::from_slice(&gas_sponsor_auth_key_bytes).map_err(AuthServerError::setup)?;

    Ok((encryption_key, management_key, relayer_admin_key, gas_sponsor_auth_key))
}

/// Setup the quote metrics recorder and sources if enabled
fn maybe_setup_quote_metrics(
    args: &Cli,
    darkpool_client: DarkpoolClient,
    price_reporter: PriceReporterClient,
) -> Option<Arc<QuoteComparisonHandler>> {
    if !args.enable_quote_comparison {
        return None;
    }

    let odos_source = QuoteSource::odos_default();
    Some(Arc::new(QuoteComparisonHandler::new(vec![odos_source], darkpool_client, price_reporter)))
}

/// Parse the gas sponsor address from the CLI args
fn parse_gas_sponsor_address(args: &Cli) -> Result<Address, AuthServerError> {
    let gas_sponsor_address_bytes =
        hex::decode(&args.gas_sponsor_address).map_err(AuthServerError::setup)?;

    Ok(Address::from_slice(&gas_sponsor_address_bytes))
}

/// Create a darkpool client with the provided configuration
pub fn create_darkpool_client(
    darkpool_address: String,
    chain_id: Chain,
    rpc_url: String,
) -> Result<DarkpoolClient, String> {
    // Create the client
    DarkpoolClient::new(DarkpoolClientConfig {
        darkpool_addr: darkpool_address,
        chain: chain_id,
        rpc_url,
        private_key: PrivateKeySigner::random(),
        block_polling_interval: DEFAULT_BLOCK_POLLING_INTERVAL,
    })
    .map_err(|e| format!("Failed to create darkpool client: {e}"))
}
