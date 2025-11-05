//! Helpers for setting up the server

use std::sync::Arc;

use aes_gcm::{Aes128Gcm, KeyInit};
use alloy::hex;
use alloy::signers::k256::ecdsa::SigningKey;
use alloy_primitives::Address;
use base64::{Engine, engine::general_purpose};
use renegade_common::types::hmac::HmacKey;
use renegade_darkpool_client::DarkpoolClient;
use reqwest::Client;

use super::Server;
use super::db::{create_db_pool, create_redis_client};
use super::gas_estimation::gas_cost_sampler::GasCostSampler;
use super::rate_limiter::AuthServerRateLimiter;
use crate::bundle_store::BundleStore;
use crate::server::caching::ServerCache;
use crate::telemetry::configure_telemtry_from_args;
use crate::{Cli, error::AuthServerError};
use price_reporter_client::PriceReporterClient;

impl Server {
    /// Create a new server instance
    pub async fn setup(
        args: Cli,
        gas_sponsor_address: Address,
        malleable_match_connector_address: Address,
        bundle_store: BundleStore,
        rate_limiter: AuthServerRateLimiter,
        price_reporter_client: PriceReporterClient,
        gas_cost_sampler: Arc<GasCostSampler>,
    ) -> Result<Self, AuthServerError> {
        configure_telemtry_from_args(&args)?;

        // Setup the DB connection pool and the Redis client
        let db_pool = create_db_pool(&args.database_url).await?;
        let redis_client = create_redis_client(&args.redis_url).await?;
        let (encryption_key, management_key, relayer_admin_key, gas_sponsor_auth_key) =
            parse_auth_server_keys(&args)?;

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
