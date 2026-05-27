//! Admin websocket listener for the pool-runner service
//!
//! Subscribes to order updates from the relayer admin websocket and triggers
//! pool routing when new user orders appear in the global matching pool.

use std::sync::Arc;

use alloy::signers::local::PrivateKeySigner;
use futures::StreamExt;
use renegade_constants::GLOBAL_MATCHING_POOL;
use renegade_external_api::types::{AdminOrderUpdateMessage, ApiOrderUpdateType};
use renegade_sdk::client::RenegadeClient;
use renegade_types_core::HmacKey;
use tracing::{error, info, warn};

use crate::server::Server;

/// Listener for admin websocket order updates
pub struct AdminWebsocketListener {
    /// Admin client used to subscribe to websocket streams
    admin_client: RenegadeClient,
    /// Server handle for invoking routing logic and notifying fill waiters
    server: Arc<Server>,
}

impl AdminWebsocketListener {
    /// Create a new listener
    pub fn new(
        relayer_admin_key: &str,
        chain_id: u64,
        server: Arc<Server>,
    ) -> anyhow::Result<Self> {
        use renegade_sdk::{
            ARBITRUM_ONE_CHAIN_ID, ARBITRUM_SEPOLIA_CHAIN_ID, BASE_MAINNET_CHAIN_ID,
            BASE_SEPOLIA_CHAIN_ID, ETHEREUM_SEPOLIA_CHAIN_ID,
        };

        let hmac_key = HmacKey::from_base64_string(relayer_admin_key)
            .map_err(|e| anyhow::anyhow!("Invalid relayer admin key: {e}"))?;

        let dummy_signer = PrivateKeySigner::random();

        let admin_client = match chain_id {
            ARBITRUM_SEPOLIA_CHAIN_ID => {
                RenegadeClient::new_arbitrum_sepolia_admin(&dummy_signer, hmac_key.into())
            },
            ARBITRUM_ONE_CHAIN_ID => {
                RenegadeClient::new_arbitrum_one_admin(&dummy_signer, hmac_key.into())
            },
            BASE_SEPOLIA_CHAIN_ID => {
                RenegadeClient::new_base_sepolia_admin(&dummy_signer, hmac_key.into())
            },
            BASE_MAINNET_CHAIN_ID => {
                RenegadeClient::new_base_mainnet_admin(&dummy_signer, hmac_key.into())
            },
            ETHEREUM_SEPOLIA_CHAIN_ID => {
                RenegadeClient::new_ethereum_sepolia_admin(&dummy_signer, hmac_key.into())
            },
            _ => anyhow::bail!("Unsupported chain ID: {chain_id}"),
        }
        .map_err(|e| anyhow::anyhow!("Failed to build admin client: {e}"))?;

        Ok(Self { admin_client, server })
    }

    /// Subscribe to admin websocket streams and run forever.
    pub async fn listen(self: Arc<Self>) {
        let self_clone = self.clone();
        let order_handle = tokio::spawn(async move {
            self_clone.listen_order_updates().await;
        });

        // Wait for the order update task (it should run forever)
        let _ = order_handle.await;
        error!("Admin websocket listener ended unexpectedly");
    }

    /// Listen for admin order updates
    async fn listen_order_updates(&self) {
        info!("Subscribing to admin order updates...");

        let mut stream = match self.admin_client.subscribe_admin_order_updates().await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to subscribe to admin order updates: {e}");
                return;
            },
        };

        info!("Listening for admin order updates...");

        while let Some(message) = stream.next().await {
            self.handle_order_update(message).await;
        }

        error!("Admin order updates stream ended");
    }

    /// Route an order update to the appropriate handler
    async fn handle_order_update(&self, message: AdminOrderUpdateMessage) {
        let order_id = message.order.order.id;

        // If there is a fill waiter for this order, notify it (internal fill)
        if message.update_type == ApiOrderUpdateType::InternalFill {
            self.server.fill_waiters.notify(order_id, message.clone()).await;
        }

        // Trigger routing for new user orders entering the global pool
        if message.update_type == ApiOrderUpdateType::Created
            && message.order.matching_pool == GLOBAL_MATCHING_POOL
        {
            info!("New user order detected: {order_id}");
            let server = self.server.clone();
            let order = message.order.clone();
            tokio::spawn(async move {
                if let Err(e) = server.try_route_user_order(&order).await {
                    warn!("Route attempt failed for order {order_id}: {e}");
                }
            });
        }
    }
}
