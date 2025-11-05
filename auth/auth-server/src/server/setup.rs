//! Helpers for setting up the server

use super::Server;
use super::db::{create_db_pool, create_redis_client};
use super::gas_estimation::gas_cost_sampler::GasCostSampler;
use super::rate_limiter::AuthServerRateLimiter;

use std::{iter, sync::Arc, time::Duration};

use crate::bundle_store::BundleStore;
use crate::server::caching::ServerCache;
use crate::telemetry::configure_telemtry_from_args;
use crate::{Cli, error::AuthServerError};
use aes_gcm::Aes128Gcm;
use alloy::signers::k256::ecdsa::SigningKey;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::Address;
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
use renegade_util::on_chain::{set_external_match_fee, set_protocol_fee};
use reqwest::Client;

/// The interval at which we poll filter updates
const DEFAULT_BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(100);

impl Server {
    /// Create a new server instance
    #[allow(clippy::too_many_arguments)]
    pub async fn setup(
        args: Cli,
        bundle_store: BundleStore,
        rate_limiter: AuthServerRateLimiter,
        price_reporter_client: PriceReporterClient,
        gas_cost_sampler: Arc<GasCostSampler>,
        encryption_key: Aes128Gcm,
        management_key: HmacKey,
        relayer_admin_key: HmacKey,
        gas_sponsor_auth_key: SigningKey,
        gas_sponsor_address: Address,
        malleable_match_connector_address: Address,
    ) -> Result<Self, AuthServerError> {
        configure_telemtry_from_args(&args)?;

        // Setup the DB connection pool and the Redis client
        let db_pool = create_db_pool(&args.database_url).await?;
        let redis_client = create_redis_client(&args.redis_url).await?;

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
            metrics_sampling_rate: args
                .metrics_sampling_rate
                .unwrap_or(1.0 /* default no sampling */),
            gas_sponsor_address,
            malleable_match_connector_address,
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
pub async fn setup_token_mapping(args: &Cli) -> Result<(), AuthServerError> {
    let chain_id = args.chain_id;
    let token_remap_file = args.token_remap_file.clone();
    tokio::task::spawn_blocking(move || setup_token_remaps(token_remap_file, chain_id))
        .await
        .unwrap()
        .map_err(AuthServerError::setup)
}

/// Set the external match fees & protocol fee
pub async fn set_external_match_fees(
    darkpool_client: &DarkpoolClient,
) -> Result<(), AuthServerError> {
    let protocol_fee = darkpool_client.get_protocol_fee().await.map_err(AuthServerError::setup)?;
    set_protocol_fee(protocol_fee);

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
