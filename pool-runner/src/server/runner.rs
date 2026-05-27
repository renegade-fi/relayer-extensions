//! Core pool-routing flow: select a managed MM pool, assign the user order,
//! await a fill, and reassign back to the global pool.

use std::time::{Duration, Instant};

use renegade_constants::GLOBAL_MATCHING_POOL;
use renegade_external_api::types::ApiAdminOrder;
use renegade_types_core::Token;
use tracing::{error, info, warn};

use crate::{
    config::ManagedPool,
    metrics::{
        record_fill_latency, record_reassign_attempt, record_reassign_failure,
        record_reassign_success, record_reassign_timeout,
    },
    server::Server,
};

/// Timeout for awaiting a fill after assigning to a managed pool
const FILL_TIMEOUT: Duration = Duration::from_secs(30);

// --- Pool selection --- //

/// Select the first managed pool that handles the given order.
///
/// A pool is eligible when:
/// - Its `base_tickers` list includes the order's base ticker, AND
/// - The order's USD value is within [`min_value_usd`, `max_value_usd`]
///   (no upper bound when `max_value_usd` is `None`).
pub fn select_managed_pool<'a>(
    pools: &'a [ManagedPool],
    base_ticker: &str,
    value_usd: f64,
) -> Option<&'a ManagedPool> {
    pools.iter().find(|p| {
        p.base_tickers.iter().any(|t| t == base_ticker)
            && value_usd >= p.min_value_usd
            && p.max_value_usd.is_none_or(|max| value_usd <= max)
    })
}

// --- Routing flow --- //

impl Server {
    /// Attempt to route a user order into a managed MM pool.
    pub(crate) async fn try_route_user_order(
        &self,
        user_order: &ApiAdminOrder,
    ) -> anyhow::Result<()> {
        let order_id = user_order.order.id;

        // Resolve the base token and its ticker
        let (base_ticker, value_usd) = self.resolve_order_metadata(user_order).await?;

        // Select a managed pool for this order
        let pool = select_managed_pool(&self.config.managed_pools, &base_ticker, value_usd)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No managed pool for order {order_id} \
                     (ticker={base_ticker}, value=${value_usd:.2})"
                )
            })?;

        let pool_name = pool.name.clone();
        record_reassign_attempt(&pool_name);

        info!(
            "Routing order {order_id} (ticker={base_ticker}, value=${value_usd:.2}) \
             to pool {pool_name}"
        );

        // Execute the assign → fill → reassign flow
        let result = self.execute_pool_match(user_order, &pool_name).await;

        match &result {
            Ok(()) => {
                record_reassign_success(&pool_name);
                info!("Order {order_id} successfully matched in pool {pool_name}");
            },
            Err(e) if is_timeout_error(e) => {
                record_reassign_timeout(&pool_name);
                warn!("Fill timeout for order {order_id} in pool {pool_name}");
            },
            Err(e) => {
                record_reassign_failure(&pool_name, &e.to_string());
                warn!("Match failed for order {order_id} in pool {pool_name}: {e}");
            },
        }

        result
    }

    /// Assign the user order to the managed pool, await a fill, then reassign
    /// back to the global pool.
    async fn execute_pool_match(
        &self,
        user_order: &ApiAdminOrder,
        pool_name: &str,
    ) -> anyhow::Result<()> {
        let user_order_id = user_order.order.id;

        // Register a fill waiter for this order before assigning
        let fill_rx = self.fill_waiters.register(user_order_id).await;

        let assign_time = Instant::now();

        // Assign user order into the managed pool
        info!("Assigning order {user_order_id} to pool {pool_name}");
        if let Err(e) = self
            .admin_client
            .admin_assign_order_to_pool(user_order_id, pool_name.to_string())
            .await
        {
            self.fill_waiters.remove(user_order_id).await;
            return Err(anyhow::anyhow!("Failed to assign order to pool: {e}"));
        }

        // Await fill with timeout
        let fill_result = tokio::time::timeout(FILL_TIMEOUT, fill_rx).await;

        // Always reassign back to global pool
        info!("Reassigning order {user_order_id} back to global pool");
        if let Err(e) = self
            .admin_client
            .admin_assign_order_to_pool(user_order_id, GLOBAL_MATCHING_POOL.to_string())
            .await
        {
            error!("Failed to reassign order {user_order_id} to global pool: {e}");
        }

        // Process the fill result
        match fill_result {
            Ok(Ok(_fill_message)) => {
                let latency_ms = assign_time.elapsed().as_millis() as f64;
                record_fill_latency(pool_name, latency_ms);
                self.fill_waiters.remove(user_order_id).await;
                Ok(())
            },
            Ok(Err(_)) => {
                self.fill_waiters.remove(user_order_id).await;
                Err(anyhow::anyhow!("Fill waiter channel closed unexpectedly"))
            },
            Err(_timeout) => {
                self.fill_waiters.remove(user_order_id).await;
                Err(timeout_error(pool_name))
            },
        }
    }

    /// Resolve the base ticker and USD value of an order.
    async fn resolve_order_metadata(
        &self,
        user_order: &ApiAdminOrder,
    ) -> anyhow::Result<(String, f64)> {
        let order = &user_order.order;
        let matchable_amount = user_order.matchable_amount;

        if matchable_amount == 0 {
            return Err(anyhow::anyhow!(
                "Order {} has zero matchable amount",
                order.id
            ));
        }

        let in_token = Token::from_alloy_address(&order.order.intent.in_token);
        let out_token = Token::from_alloy_address(&order.order.intent.out_token);
        let usdc = Token::usdc();

        // Determine which token is the base token (non-USDC)
        let (base_token, quote_is_in) = if in_token == usdc && out_token != usdc {
            (out_token, true) // buying base with USDC
        } else if in_token != usdc && out_token == usdc {
            (in_token, false) // selling base for USDC
        } else {
            return Err(anyhow::anyhow!(
                "Cannot resolve base token for order {}",
                order.id
            ));
        };

        let base_ticker = base_token
            .get_ticker()
            .ok_or_else(|| anyhow::anyhow!("Unknown base token for order {}", order.id))?
            .to_string();

        // Compute USD value of the matchable amount
        let value_usd = if quote_is_in {
            // in_token is USDC → matchable amount is already in USD
            usdc.convert_to_decimal(matchable_amount)
        } else {
            // in_token is the base token → need price
            let base_price = self
                .price_reporter
                .get_price(&base_token.get_addr(), base_token.get_chain())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get price for {base_ticker}: {e}"))?;
            let matchable_decimal = base_token.convert_to_decimal(matchable_amount);
            matchable_decimal * base_price
        };

        Ok((base_ticker, value_usd))
    }
}

// --- Helpers --- //

/// Returns `true` if the error is a fill timeout
fn is_timeout_error(e: &anyhow::Error) -> bool {
    e.to_string().contains("Fill timeout")
}

/// Construct a timeout error for the given pool
fn timeout_error(pool_name: &str) -> anyhow::Error {
    anyhow::anyhow!("Fill timeout after {}s in pool {pool_name}", FILL_TIMEOUT.as_secs())
}

// --- Unit tests --- //

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ManagedPool;

    fn make_pool(name: &str, tickers: &[&str], min: f64, max: Option<f64>) -> ManagedPool {
        ManagedPool {
            name: name.to_string(),
            base_tickers: tickers.iter().map(|s| s.to_string()).collect(),
            min_value_usd: min,
            max_value_usd: max,
        }
    }

    #[test]
    fn test_select_pool_asset_and_size_match() {
        let pools = vec![make_pool("eth-pool", &["ETH"], 10.0, Some(10_000.0))];
        let result = select_managed_pool(&pools, "ETH", 100.0);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "eth-pool");
    }

    #[test]
    fn test_select_pool_unsupported_asset() {
        let pools = vec![make_pool("eth-pool", &["ETH"], 10.0, Some(10_000.0))];
        let result = select_managed_pool(&pools, "BTC", 100.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_select_pool_undersize() {
        let pools = vec![make_pool("eth-pool", &["ETH"], 10.0, Some(10_000.0))];
        let result = select_managed_pool(&pools, "ETH", 5.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_select_pool_oversize() {
        let pools = vec![make_pool("eth-pool", &["ETH"], 10.0, Some(10_000.0))];
        let result = select_managed_pool(&pools, "ETH", 20_000.0);
        assert!(result.is_none());
    }
}
