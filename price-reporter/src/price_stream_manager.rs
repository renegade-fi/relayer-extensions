//! Manages exchange connections and price streams

use std::{collections::HashMap, sync::Arc, time::Duration};

use dashmap::DashMap;
use renegade_common::types::{exchange::Exchange, price::Price};
use tokio::{
    sync::{watch::channel, RwLock},
    time::Instant,
};
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

use crate::{
    errors::ServerError,
    exchanges::{
        connect_exchange, connection::ExchangeConnection, error::ExchangeConnectionError,
        ExchangeConnectionsConfig,
    },
    utils::{
        metrics::record_price_update_latency, ClosureSender, PairInfo, PriceReceiver, PriceSender,
        PriceStream, SharedPriceStreams, CONN_RETRY_DELAY_MS, KEEPALIVE_INTERVAL_MS,
        MAX_CONN_RETRIES, MAX_CONN_RETRY_WINDOW_MS,
    },
};

// -------------
// | Constants |
// -------------

/// The price for a unit pair
///
/// A unit pair is a pair in which the base and quote tokens are the same.
///
/// We simply send a price of 1.0 for such pairs
const UNIT_PAIR_PRICE: f64 = 1.0;
/// The interval at which to refresh the unit price
const UNIT_PRICE_REFRESH_INTERVAL_MS: u64 = 1_000; // 1 second

// ---------
// | Types |
// ---------

/// A type alias for a shareable map of the last timestamp at which a price was
/// updated, indexed by the (source, base, quote) tuple
type SharedPriceUpdateTimestamps = Arc<DashMap<PairInfo, Instant>>;

/// A map of price streams from exchanges maintained by the server,
/// shared across all connections
#[derive(Clone)]
pub(crate) struct GlobalPriceStreams {
    /// A thread-safe map of price streams, indexed by the (source, base, quote)
    /// tuple
    pub price_streams: SharedPriceStreams,
    /// A channel to send closure signals from the price stream tasks
    pub closure_channel: ClosureSender,
    /// A thread-safe map of the last timestamp at which a price was updated,
    /// indexed by the (source, base, quote) tuple
    pub price_update_timestamps: SharedPriceUpdateTimestamps,
}

impl GlobalPriceStreams {
    /// Instantiate a new global price streams map
    pub fn new(closure_channel: ClosureSender) -> Self {
        let price_streams = Arc::new(RwLock::new(HashMap::new()));
        let price_update_timestamps = Arc::new(DashMap::new());
        Self { price_streams, closure_channel, price_update_timestamps }
    }

    /// Attempt to add a price stream to the global map, returning the price
    /// receiver if one already exists
    pub async fn maybe_add_price_stream(
        &self,
        pair_info: PairInfo,
        price_rx: PriceReceiver,
    ) -> Option<PriceReceiver> {
        let mut price_streams = self.price_streams.write().await;

        if let Some(stream_rx) = price_streams.get(&pair_info).cloned() {
            return Some(stream_rx);
        }

        price_streams.insert(pair_info, price_rx);

        None
    }

    /// Remove a price stream from the global map
    pub async fn remove_price_stream(&self, pair_info: PairInfo) {
        self.price_streams.write().await.remove(&pair_info);
    }

    /// Initialize a price stream for the given pair info
    pub async fn init_price_stream(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceReceiver, ServerError> {
        pair_info.validate_subscription().await?;

        // Create a shared channel into which we forward streamed prices
        let (price_tx, price_rx) = channel(Price::default());

        if let Some(stream_rx) =
            self.maybe_add_price_stream(pair_info.clone(), price_rx.clone()).await
        {
            // If a price receiver entry already exists, return it.
            // This prevents a race condition where two concurrent
            // calls with the same `pair_info` result in duplicate price stream
            // tasks being spawned.
            return Ok(stream_rx);
        }

        info!("Initializing price stream for {}", pair_info.to_topic());

        // Spawn a task responsible for forwarding prices into the broadcast channel &
        // sending keepalive messages to the exchange
        let global_price_streams = self.clone();
        tokio::spawn(async move {
            let res = Self::price_stream_task(
                config,
                pair_info.clone(),
                price_tx,
                &global_price_streams.price_update_timestamps,
            )
            .await;

            global_price_streams.remove_price_stream(pair_info).await;
            global_price_streams.closure_channel.send(res).unwrap()
        });

        // Return a handle to the broadcast channel stream
        Ok(price_rx)
    }

    /// The task responsible for streaming prices from the exchange
    async fn price_stream_task(
        config: ExchangeConnectionsConfig,
        pair_info: PairInfo,
        price_tx: PriceSender,
        price_update_timestamps: &SharedPriceUpdateTimestamps,
    ) -> Result<(), ServerError> {
        if pair_info.is_unit_pair() {
            Self::stream_unit_pair_price(&price_tx).await;
            return Ok(());
        }

        // Connect to the pair on the specified exchange
        let mut retry_timestamps = Vec::new();
        let mut conn = Self::connect_with_retries(
            &pair_info,
            &config,
            &mut retry_timestamps,
            price_update_timestamps,
        )
        .await?;

        loop {
            match Self::manage_connection(&mut conn, &price_tx, &pair_info, price_update_timestamps)
                .await
            {
                Ok(()) => {},
                Err(e) => {
                    conn = Self::exhaust_retries(
                        e,
                        &pair_info,
                        &config,
                        &mut retry_timestamps,
                        price_update_timestamps,
                    )
                    .await?;
                },
            }
        }
    }

    /// Stream a unit pair price
    ///
    /// We simply send a price of 1.0 in a loop with a delay. This will keep the
    /// price "fresh" as measured by consumers in this service and via the API.
    async fn stream_unit_pair_price(price_tx: &PriceSender) {
        let refresh_interval = Duration::from_millis(UNIT_PRICE_REFRESH_INTERVAL_MS);
        loop {
            let _ = price_tx.send(UNIT_PAIR_PRICE);
            tokio::time::sleep(refresh_interval).await;
        }
    }

    /// Manages an exchange connection, sending keepalive messages and
    /// forwarding prices to the price receiver
    async fn manage_connection(
        conn: &mut Box<dyn ExchangeConnection>,
        price_tx: &PriceSender,
        pair_info: &PairInfo,
        price_update_timestamps: &SharedPriceUpdateTimestamps,
    ) -> Result<(), ServerError> {
        let delay = tokio::time::sleep(Duration::from_millis(KEEPALIVE_INTERVAL_MS));
        tokio::pin!(delay);

        loop {
            tokio::select! {
                // Send a keepalive message to the exchange
                _ = &mut delay => {
                    conn.send_keepalive().await.map_err(ServerError::ExchangeConnection)?;
                    delay.as_mut().reset(Instant::now() + Duration::from_millis(KEEPALIVE_INTERVAL_MS));
                }

                // Forward the next price into the broadcast channel
                maybe_price_res = conn.next() => match maybe_price_res {
                    Some(price_res) => {
                        let price = price_res.map_err(ServerError::ExchangeConnection)?;
                        let _ = price_tx.send(price);

                        // Record the update timestamp
                        Self::record_price_update_ts(pair_info, price_update_timestamps).await;
                    }
                    None => {
                        let stream_closure_msg = format!("Price stream for {} has closed", pair_info.to_topic());
                        return Err(ServerError::ExchangeConnection(ExchangeConnectionError::ConnectionHangup(stream_closure_msg)))
                    }
                }
            }
        }
    }

    /// Record a new update timestamp for the given price stream, emitting a
    /// metric tracking the latency since the last update
    async fn record_price_update_ts(
        pair_info: &PairInfo,
        price_update_timestamps: &SharedPriceUpdateTimestamps,
    ) {
        let update_ts = Instant::now();

        let last_update_ts = price_update_timestamps.get(pair_info);
        if let Some(last_update_ts) = last_update_ts {
            let update_latency = update_ts.duration_since(*last_update_ts).as_millis_f64();
            record_price_update_latency(pair_info, update_latency);
        }

        price_update_timestamps.insert(pair_info.clone(), update_ts);
    }

    /// Initialize an exchange connection, retrying if necessary
    async fn connect_with_retries(
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
        price_update_timestamps: &SharedPriceUpdateTimestamps,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        // Attempt to connect to the pair on the specified exchange
        match connect_exchange(pair_info.clone(), config)
            .await
            .map_err(ServerError::ExchangeConnection)
        {
            Ok(conn) => Ok(conn),
            Err(e) => {
                warn!("Error in initial connection to {}: {}", pair_info.to_topic(), e);
                Self::exhaust_retries(
                    e,
                    pair_info,
                    config,
                    retry_timestamps,
                    price_update_timestamps,
                )
                .await
            },
        }
    }

    /// Attempt to re-establish an erroring exchange connection, exhausting
    /// retries if necessary
    async fn exhaust_retries(
        mut prev_err: ServerError,
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
        price_update_timestamps: &SharedPriceUpdateTimestamps,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        warn!("Error in connection to {}: {}", pair_info.to_topic(), prev_err);
        let exchange = pair_info.exchange;
        loop {
            prev_err = match Self::retry_connection(
                pair_info,
                config,
                retry_timestamps,
                price_update_timestamps,
            )
            .await
            {
                Ok(conn) => return Ok(conn),
                Err(ServerError::ExchangeConnection(ExchangeConnectionError::MaxRetries(
                    exchange,
                ))) => {
                    // Return the original error if we've exhausted retries
                    error!("Exhausted retries for {}", exchange);
                    return Err(prev_err);
                },
                Err(e) => {
                    warn!("Failed to reconnect to {exchange}: {e}");
                    e
                },
            };
        }
    }

    /// Retries an exchange connection after it has failed.
    ///
    /// Mirrors https://github.com/renegade-fi/renegade/blob/main/workers/price-reporter/src/reporter.rs#L470
    async fn retry_connection(
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
        price_update_timestamps: &SharedPriceUpdateTimestamps,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        warn!("Retrying connection for {}", pair_info.to_topic());

        let last_update_ts = price_update_timestamps.get(pair_info);
        if let Some(last_update_ts) = last_update_ts {
            let time_since_last_update =
                Instant::now().duration_since(*last_update_ts).as_millis_f64();

            warn!("Time since last update: {time_since_last_update}ms");
        }

        // Increment the retry count and filter out old requests
        let now = Instant::now();
        let retry_window = Duration::from_millis(MAX_CONN_RETRY_WINDOW_MS);
        retry_timestamps.retain(|ts| now.duration_since(*ts) < retry_window);

        // Add the current timestamp to the set of retries
        retry_timestamps.push(now);
        if retry_timestamps.len() >= MAX_CONN_RETRIES {
            return Err(ServerError::ExchangeConnection(ExchangeConnectionError::MaxRetries(
                pair_info.exchange,
            )));
        }

        // Add delay before retrying
        tokio::time::sleep(Duration::from_millis(CONN_RETRY_DELAY_MS)).await;

        // Reconnect
        connect_exchange(pair_info.clone(), config).await.map_err(ServerError::ExchangeConnection)
    }

    /// Returns a tuple of (canonicalized pair info, requires quote conversion),
    /// if needed
    fn normalize_pair_info(&self, pair_info: PairInfo) -> Result<(PairInfo, bool), ServerError> {
        if pair_info.exchange != Exchange::Renegade {
            return Ok((pair_info, false));
        }

        let base_mint = pair_info.base_token().get_addr();
        let new_pair_info = PairInfo::new_canonical_exchange(&base_mint)?;
        let requires_conversion = pair_info.requires_quote_conversion()?;
        Ok((new_pair_info, requires_conversion))
    }

    /// Fetch a price stream for the given pair info from the global map
    pub async fn get_or_create_price_stream(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceStream, ServerError> {
        let (normalized_pair_info, requires_conversion) =
            self.normalize_pair_info(pair_info.clone())?;

        let price_rx =
            self.get_or_create_price_receiver(normalized_pair_info.clone(), config.clone()).await?;
        let stream = if requires_conversion {
            let conversion_rx = self.quote_conversion_stream(normalized_pair_info, config).await?;
            PriceStream::new_with_conversion(price_rx.into(), conversion_rx.into())
        } else {
            PriceStream::new(price_rx.into())
        };

        Ok(stream)
    }

    /// Get a quote conversion stream for a given exchange
    ///
    /// Currently we only need to convert USDT -> USDC for `Renegade` prices, so
    /// this method does not configure the conversion tokens.
    async fn quote_conversion_stream(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceReceiver, ServerError> {
        let conversion_pair = pair_info.get_conversion_pair()?;
        let conversion_rx = self.get_or_create_price_receiver(conversion_pair, config).await?;
        Ok(conversion_rx)
    }

    /// Get a price receiver for the given pair or create a new stream
    async fn get_or_create_price_receiver(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceReceiver, ServerError> {
        let price_streams = self.price_streams.read().await;
        if let Some(stream_rx) = price_streams.get(&pair_info).cloned() {
            return Ok(stream_rx);
        }

        drop(price_streams);

        self.init_price_stream(pair_info, config).await
    }
}
