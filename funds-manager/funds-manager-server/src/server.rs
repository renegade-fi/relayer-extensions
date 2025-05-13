//! Defines the server which encapsulates all dependencies for funds manager
//! execution

use std::{collections::HashMap, error::Error, sync::Arc};

use aws_config::{BehaviorVersion, Region, SdkConfig};
use funds_manager_api::quoters::ExecutionQuote;
use renegade_common::types::{chain::Chain, hmac::HmacKey, token::Token};
use renegade_config::setup_token_remaps;

use crate::{
    cli::{ChainClients, Cli},
    db::{create_db_pool, DbPool},
    error::FundsManagerError,
};

// -------------
// | Constants |
// -------------

/// The default region in which to provision secrets manager secrets
const DEFAULT_REGION: &str = "us-east-2";

/// The server
#[derive(Clone)]
pub(crate) struct Server {
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
    /// The AWS config
    pub aws_config: SdkConfig,
    /// The HMAC key for custody endpoint authentication
    pub hmac_key: Option<HmacKey>,
    /// The HMAC key for signing quotes
    pub quote_hmac_key: HmacKey,
    /// The chain clients
    pub chain_clients: HashMap<Chain, ChainClients>,
}

impl Server {
    /// Build a server from the CLI
    pub async fn build_from_cli(args: Cli) -> Result<Self, Box<dyn Error>> {
        // Parse an AWS config
        let aws_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(DEFAULT_REGION))
            .load()
            .await;

        let chain_configs = args.parse_chain_configs(&aws_config).await?;

        for chain in chain_configs.keys() {
            let chain = *chain;
            tokio::task::spawn_blocking(move || {
                setup_token_remaps(None /* token_remap_file */, chain)
            })
            .await
            .unwrap()?;
        }

        let hmac_key = args.get_hmac_key();
        let quote_hmac_key = args.get_quote_hmac_key();

        // Create a database connection pool using bb8
        let db_pool = create_db_pool(&args.db_url).await?;
        let arc_pool = Arc::new(db_pool);

        let usdc_mint = Token::usdc().get_addr();

        let mut chain_clients = HashMap::new();
        for (chain, config) in chain_configs {
            let clients = config
                .build_clients(
                    chain,
                    args.fireblocks_api_key.clone(),
                    args.fireblocks_api_secret.clone(),
                    arc_pool.clone(),
                    aws_config.clone(),
                    &usdc_mint,
                )
                .await?;

            chain_clients.insert(chain, clients);
        }

        Ok(Server { db_pool: arc_pool, aws_config, hmac_key, quote_hmac_key, chain_clients })
    }

    /// Sign a quote using the quote HMAC key and returns the signature as a
    /// hex string
    pub fn sign_quote(&self, quote: &ExecutionQuote) -> Result<String, FundsManagerError> {
        let canonical_string = quote.to_canonical_string();
        let sig = self.quote_hmac_key.compute_mac(canonical_string.as_bytes());
        let signature = hex::encode(sig);
        Ok(signature)
    }
}
