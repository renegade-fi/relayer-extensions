//! Helpers for setting up the server

use super::Server;
use super::db::{create_db_pool, create_redis_client};
use super::gas_estimation::gas_cost_sampler::GasCostSampler;
use super::rate_limiter::AuthServerRateLimiter;

use std::{iter, sync::Arc, time::Duration};

use crate::bundle_store::BundleStore;
use crate::server::caching::ServerCache;
use crate::telemetry::configure_telemetry_from_args;
use crate::{Cli, error::AuthServerError};
use aes_gcm::{Aes128Gcm, KeyInit};
use alloy::hex;
use alloy::signers::k256::ecdsa::SigningKey;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::Address;
use base64::{Engine, engine::general_purpose};
use price_reporter_client::{PriceReporterClient, PriceReporterClientConfig};
use renegade_config::setup_token_remaps;
use renegade_constants::NATIVE_ASSET_ADDRESS;
use renegade_darkpool_client::DarkpoolClient;
use renegade_darkpool_client::client::DarkpoolClientConfig;
use renegade_system_clock::SystemClock;
use renegade_types_core::{Chain, get_all_tokens};
use renegade_types_core::{HmacKey, Token};
use renegade_util::hex::address_from_hex_string;
use renegade_util::on_chain::set_protocol_fee;
use reqwest::Client;
use tokio_util::sync::CancellationToken;

/// The interval at which we poll filter updates
const DEFAULT_BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(100);

impl Server {
    /// Create a new server instance
    pub async fn setup(
        args: Cli,
        system_clock: &SystemClock,
    ) -> Result<(Self, CancellationToken), AuthServerError> {
        configure_telemetry_from_args(&args)?;
        setup_token_mapping(&args).await?;

        // Create the darkpool client
        let darkpool_client = create_darkpool_client(
            &args.darkpool_address,
            &args.permit2_address,
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

        let price_reporter_client = PriceReporterClient::new(PriceReporterClientConfig {
            base_url: args.price_reporter_url.clone(),
            ..Default::default()
        })?;

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

        // CHAIN EVENTS LISTENER TEMPORARILY DISABLED
        // Start the on-chain event listener
        // let chain_listener_config = OnChainEventListenerConfig {
        //     chain: args.chain_id,
        //     gas_sponsor_address,
        //     websocket_addr: args.eth_websocket_addr.clone(),
        //     bundle_store: bundle_store.clone(),
        //     rate_limiter: rate_limiter.clone(),
        //     price_reporter_client: price_reporter_client.clone(),
        //     gas_cost_sampler: gas_cost_sampler.clone(),
        //     darkpool_client: darkpool_client.clone(),
        // };
        // let mut chain_listener = OnChainEventListener::new(chain_listener_config)
        //     .expect("failed to build on-chain event listener");
        // chain_listener.start().expect("failed to start on-chain event listener");
        let chain_listener_cancellation_token = CancellationToken::new();
        // chain_listener.watch(chain_listener_cancellation_token.clone());

        let server = Self {
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
            metrics_sampling_rate: args
                .metrics_sampling_rate
                .unwrap_or(1.0 /* default no sampling */),
            gas_sponsor_address,
            gas_sponsor_auth_key,
            price_reporter_client,
            gas_cost_sampler,
            min_sponsored_order_quote_amount: args.min_sponsored_order_quote_amount,
            bundle_store,
        };
        Ok((server, chain_listener_cancellation_token))
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
    let tokens: Vec<Token> = get_all_tokens()
        .into_iter()
        .chain(iter::once(Token::from_addr(NATIVE_ASSET_ADDRESS)))
        .collect();

    let usdc = Token::usdc().get_alloy_address();
    for token in tokens {
        if token.get_alloy_address() == usdc {
            continue;
        }

        // Fetch the fee override from the contract
        let addr = token.get_alloy_address();
        let fee =
            darkpool_client.get_protocol_fee(addr, usdc).await.map_err(AuthServerError::setup)?;

        // Write the fee into the mapping
        set_protocol_fee(&addr, &usdc, fee);
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

/// Parse the gas sponsor address from the CLI args
fn parse_gas_sponsor_address(args: &Cli) -> Result<Address, AuthServerError> {
    address_from_hex_string(&args.gas_sponsor_address).map_err(AuthServerError::setup)
}

/// Create a darkpool client with the provided configuration
pub fn create_darkpool_client(
    darkpool_address: &str,
    permit2_address: &str,
    chain: Chain,
    rpc_url: String,
) -> Result<DarkpoolClient, String> {
    let darkpool_addr = address_from_hex_string(&darkpool_address)?;
    let permit2_addr = address_from_hex_string(&permit2_address)?;

    // Create the client
    DarkpoolClient::new(DarkpoolClientConfig {
        darkpool_addr,
        permit2_addr,
        chain,
        rpc_url,
        private_key: PrivateKeySigner::random(),
        block_polling_interval: DEFAULT_BLOCK_POLLING_INTERVAL,
    })
    .map_err(|e| format!("Failed to create darkpool client: {e}"))
}
