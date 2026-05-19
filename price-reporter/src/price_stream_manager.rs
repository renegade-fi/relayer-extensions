//! Manages exchange connections and price streams

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use renegade_types_core::{Exchange, Price};
use tokio::{
    sync::{RwLock, watch::channel},
    time::Instant,
};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::{
    errors::ServerError,
    exchanges::{
        ExchangeConnectionsConfig, connect_exchange, connection::ExchangeConnection,
        error::ExchangeConnectionError,
    },
    log_task,
    logger::{Outcome, Task},
    utils::{
        CONN_RETRY_DELAY, ClosureSender, FEED_AGE_EMIT_INTERVAL, HEARTBEAT_INTERVAL,
        HEARTBEAT_REPLAY_WARN_AGE, KEEPALIVE_INTERVAL, MAX_CONN_RETRIES, MAX_CONN_RETRY_WINDOW,
        MAX_HEARTBEAT_AGE, PairInfo, PriceReceiver, PriceSender, PriceStream,
        RATE_LIMIT_RETRY_DELAY, SUBSCRIBE_ACK_TIMEOUT, SharedPriceStreams,
    },
};

/// Tracks the wall-clock time of the most recent real price tick for a single
/// stream, independent of the heartbeat-watchdog clock (which resets on every
/// reconnect). Stored as millis-since-UNIX_EPOCH in an `AtomicU64` so the
/// emitter task and the connection-management task can share it without a
/// lock. A value of `0` means no real tick has been observed yet.
type LastRealTick = Arc<AtomicU64>;

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The price for a unit pair
///
/// A unit pair is a pair in which the base and quote tokens are the same.
///
/// We simply send a price of 1.0 for such pairs
const UNIT_PAIR_PRICE: f64 = 1.0;
/// The interval at which to refresh the unit price
const UNIT_PRICE_REFRESH_INTERVAL_MS: u64 = 1_000; // 1 second

/// A map of price streams from exchanges maintained by the server,
/// shared across all connections
#[derive(Clone)]
pub(crate) struct GlobalPriceStreams {
    /// A thread-safe map of price streams, indexed by the (source, base, quote)
    /// tuple
    pub price_streams: SharedPriceStreams,
    /// A channel to send closure signals from the price stream tasks
    pub closure_channel: ClosureSender,
}

impl GlobalPriceStreams {
    /// Instantiate a new global price streams map
    pub fn new(closure_channel: ClosureSender) -> Self {
        Self { price_streams: Arc::new(RwLock::new(HashMap::new())), closure_channel }
    }

    /// Attempt to add a price stream to the global map, returning the price
    /// receiver if one already exists
    pub async fn maybe_add_price_stream(
        &self,
        pair_info: PairInfo,
        price_rx: PriceReceiver,
        cancel_token: CancellationToken,
    ) -> Option<PriceReceiver> {
        let mut price_streams = self.price_streams.write().await;

        if let Some((stream_rx, _)) = price_streams.get(&pair_info).cloned() {
            return Some(stream_rx);
        }

        price_streams.insert(pair_info, (price_rx, cancel_token));

        None
    }

    /// Remove a price stream from the global map, cancelling its task
    pub async fn remove_price_stream(&self, pair_info: PairInfo) {
        if let Some((_, cancel_token)) = self.price_streams.write().await.remove(&pair_info) {
            cancel_token.cancel();
        }
    }

    /// Cancel all streams whose PairInfo is not in the desired set
    pub async fn cancel_removed_streams(&self, desired: &HashSet<PairInfo>) {
        let to_cancel: Vec<PairInfo> = {
            let streams = self.price_streams.read().await;
            streams.keys().filter(|k| !desired.contains(k)).cloned().collect()
        };

        if !to_cancel.is_empty() {
            let mut streams = self.price_streams.write().await;
            for pair_info in to_cancel {
                log_task!(
                    Task::PriceStream,
                    Outcome::Ok,
                    subject = %pair_info.to_topic(),
                    "cancelling removed price stream"
                );
                if let Some((_, cancel_token)) = streams.remove(&pair_info) {
                    cancel_token.cancel();
                }
            }
        }
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
        let cancel_token = CancellationToken::new();

        if let Some(stream_rx) = self
            .maybe_add_price_stream(pair_info.clone(), price_rx.clone(), cancel_token.clone())
            .await
        {
            // If a price receiver entry already exists, return it.
            // This prevents a race condition where two concurrent
            // calls with the same `pair_info` result in duplicate price stream
            // tasks being spawned.
            return Ok(stream_rx);
        }

        log_task!(
            Task::PriceStream,
            Outcome::Started,
            subject = %pair_info.to_topic(),
            "initializing price stream"
        );

        // Spawn a task responsible for forwarding prices into the broadcast channel &
        // sending keepalive messages to the exchange
        let global_price_streams = self.clone();
        let task_cancel = cancel_token.clone();
        tokio::spawn(async move {
            let res =
                Self::price_stream_task(config, pair_info.clone(), price_tx, task_cancel).await;
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
        cancel_token: CancellationToken,
    ) -> Result<(), ServerError> {
        if pair_info.is_unit_pair() {
            Self::stream_unit_pair_price(&price_tx).await;
            return Ok(());
        }

        // Shared real-tick clock. Updated only when a genuine exchange tick
        // arrives in `manage_connection`. Survives reconnects, so the
        // `exchange_last_update_age_seconds` gauge reflects true upstream
        // staleness regardless of socket churn.
        let last_real_tick: LastRealTick = Arc::new(AtomicU64::new(0));

        // Spawn the per-feed age-gauge emitter. Bound it to the same cancel
        // token as the parent task so it shuts down cleanly.
        let emitter_cancel = cancel_token.clone();
        let emitter_pair = pair_info.clone();
        let emitter_last_tick = last_real_tick.clone();
        tokio::spawn(async move {
            Self::emit_feed_age_loop(emitter_pair, emitter_last_tick, emitter_cancel).await;
        });

        // Connect to the pair on the specified exchange
        let mut retry_timestamps = Vec::new();
        let mut conn =
            Self::connect_with_retries(&pair_info, &config, &mut retry_timestamps).await?;

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    log_task!(
                        Task::PriceStream,
                        Outcome::Ok,
                        subject = %pair_info.to_topic(),
                        "price stream cancelled"
                    );
                    return Ok(());
                }
                res = Self::manage_connection(&mut conn, &price_tx, &pair_info, &last_real_tick) => {
                    match res {
                        Ok(()) => {},
                        Err(e) => {
                            conn = Self::exhaust_retries(e, &pair_info, &config, &mut retry_timestamps)
                                .await?;
                        },
                    }
                }
            }
        }
    }

    /// Periodically emit `exchange_last_update_age_seconds` for this stream
    /// until the parent task is cancelled. The gauge value is the wall-clock
    /// age of the most recent real exchange tick; while the connection is
    /// down or stuck, the value keeps growing.
    async fn emit_feed_age_loop(
        pair_info: PairInfo,
        last_real_tick: LastRealTick,
        cancel_token: CancellationToken,
    ) {
        let topic = pair_info.to_topic();
        let mut tick = tokio::time::interval(FEED_AGE_EMIT_INTERVAL);
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => return,
                _ = tick.tick() => {
                    let last_ms = last_real_tick.load(Ordering::Relaxed);
                    // Skip emission until we've observed at least one tick;
                    // emitting `now` before the stream is up would produce
                    // misleading huge values during initial connect.
                    if last_ms == 0 {
                        continue;
                    }
                    let age_secs = now_millis().saturating_sub(last_ms) as f64 / 1000.0;
                    renegade_util::metrics::gauge!(
                        "exchange_last_update_age_seconds",
                        "pair" => topic.clone(),
                    )
                    .set(age_secs);
                }
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
        last_real_tick: &LastRealTick,
    ) -> Result<(), ServerError> {
        let keepalive_delay = tokio::time::sleep(KEEPALIVE_INTERVAL);
        let heartbeat_delay = tokio::time::sleep(HEARTBEAT_INTERVAL);
        let subscribe_ack_deadline = tokio::time::sleep(SUBSCRIBE_ACK_TIMEOUT);
        tokio::pin!(keepalive_delay);
        tokio::pin!(heartbeat_delay);
        tokio::pin!(subscribe_ack_deadline);
        let mut last_price: Option<Price> = None;
        let mut last_exchange_update = Instant::now();
        let mut received_first_tick = false;
        // Edge-triggered state for the heartbeat-replay warn. We log once
        // when the replay age first crosses `HEARTBEAT_REPLAY_WARN_AGE` and
        // once when a real tick resumes — not on every heartbeat in
        // between. Avoids log-spam on illiquid pairs that legitimately
        // tick every 60–120s.
        let mut replay_stalled = false;

        loop {
            tokio::select! {
                // Send a keepalive message to the exchange
                _ = &mut keepalive_delay => {
                    conn.send_keepalive().await.map_err(ServerError::ExchangeConnection)?;
                    keepalive_delay.as_mut().reset(Instant::now() + KEEPALIVE_INTERVAL);
                }

                // Subscribe-ack deadline: if we have not received a real tick
                // within `SUBSCRIBE_ACK_TIMEOUT` of (re)connecting, treat the
                // subscription as dead and bail to the retry loop. Catches the
                // case where the websocket handshake succeeds but the
                // subscribe message is silently dropped.
                _ = &mut subscribe_ack_deadline, if !received_first_tick => {
                    log_task!(
                        Task::Subscription,
                        Outcome::Failed,
                        subject = %pair_info.to_topic(),
                        timeout_secs = SUBSCRIBE_ACK_TIMEOUT.as_secs(),
                        "no price tick received after connect; subscription appears dead"
                    );
                    return Err(ServerError::ExchangeConnection(
                        ExchangeConnectionError::ConnectionHangup(format!(
                            "no price tick within {}s of connect for {}",
                            SUBSCRIBE_ACK_TIMEOUT.as_secs(),
                            pair_info.to_topic(),
                        )),
                    ));
                }

                // Re-send the last known price to keep downstream timestamps
                // fresh, but only if we've received a real exchange update
                // recently — otherwise treat the connection as dead.
                _ = &mut heartbeat_delay => {
                    if let Some(price) = last_price {
                        let age = last_exchange_update.elapsed();
                        if age < MAX_HEARTBEAT_AGE {
                            // Edge-trigger: log exactly once when the replay
                            // age first crosses the threshold. The matching
                            // "recovered" log fires from the price-tick arm
                            // when a real update finally arrives.
                            if !replay_stalled && age > HEARTBEAT_REPLAY_WARN_AGE {
                                log_task!(
                                    Task::Heartbeat,
                                    Outcome::Partial,
                                    subject = %pair_info.to_topic(),
                                    age_secs = age.as_secs(),
                                    "replaying cached price; no real exchange update"
                                );
                                replay_stalled = true;
                            }
                            let _ = price_tx.send(price);
                        } else {
                            log_task!(
                                Task::Heartbeat,
                                Outcome::Failed,
                                subject = %pair_info.to_topic(),
                                age_secs = age.as_secs(),
                                "no exchange update past max heartbeat age; treating as stale"
                            );
                            return Err(ServerError::ExchangeConnection(
                                ExchangeConnectionError::ConnectionHangup(
                                    format!("No exchange data for {:?}", age),
                                ),
                            ));
                        }
                    }
                    heartbeat_delay.as_mut().reset(Instant::now() + HEARTBEAT_INTERVAL);
                }

                // Forward the next price into the broadcast channel
                maybe_price_res = conn.next() => match maybe_price_res {
                    Some(price_res) => {
                        let price = price_res.map_err(ServerError::ExchangeConnection)?;
                        // Edge-trigger: pair the earlier "replaying cached
                        // price" warn with a single recovery log when a
                        // real tick resumes. Captures the stall duration
                        // before we reset `last_exchange_update`.
                        if replay_stalled {
                            log_task!(
                                Task::Heartbeat,
                                Outcome::Ok,
                                subject = %pair_info.to_topic(),
                                stalled_for_secs = last_exchange_update.elapsed().as_secs(),
                                "real exchange update resumed"
                            );
                            replay_stalled = false;
                        }
                        let _ = price_tx.send(price);
                        last_price = Some(price);
                        last_exchange_update = Instant::now();
                        received_first_tick = true;
                        last_real_tick.store(now_millis(), Ordering::Relaxed);
                        heartbeat_delay.as_mut().reset(Instant::now() + HEARTBEAT_INTERVAL);
                        renegade_util::metrics::counter!("exchange_updates", "pair" => pair_info.to_topic()).increment(1);
                    }
                    None => {
                        let stream_closure_msg = format!("Price stream for {} has closed", pair_info.to_topic());
                        return Err(ServerError::ExchangeConnection(ExchangeConnectionError::ConnectionHangup(stream_closure_msg)))
                    }
                }
            }
        }
    }

    /// Initialize an exchange connection, retrying if necessary
    async fn connect_with_retries(
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        // Attempt to connect to the pair on the specified exchange
        match connect_exchange(pair_info.clone(), config)
            .await
            .map_err(ServerError::ExchangeConnection)
        {
            Ok(conn) => Ok(conn),
            Err(e) => Self::exhaust_retries(e, pair_info, config, retry_timestamps).await,
        }
    }

    /// Attempt to re-establish an erroring exchange connection, exhausting
    /// retries if necessary
    async fn exhaust_retries(
        mut prev_err: ServerError,
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        log_task!(
            Task::ExchangeConnection,
            Outcome::Failed,
            subject = %pair_info.to_topic(),
            error = %prev_err,
            "error in connection"
        );
        let retry_start = Instant::now();
        let attempts_before_loop = retry_timestamps.len();
        let exchange = pair_info.exchange;
        loop {
            // Add delay before retrying
            if prev_err.is_rate_limit_error() {
                // We were rate limited, so we wait longer before retrying
                log_task!(
                    Task::ExchangeConnection,
                    Outcome::Retrying,
                    subject = %pair_info.to_topic(),
                    delay_secs = RATE_LIMIT_RETRY_DELAY.as_secs(),
                    "waiting before retry due to rate limit"
                );

                tokio::time::sleep(RATE_LIMIT_RETRY_DELAY).await;
            } else {
                tokio::time::sleep(CONN_RETRY_DELAY).await;
            }

            prev_err = match Self::retry_connection(pair_info, config, retry_timestamps).await {
                Ok(conn) => {
                    log_task!(
                        Task::ExchangeConnection,
                        Outcome::Ok,
                        subject = %pair_info.to_topic(),
                        attempts = retry_timestamps.len() - attempts_before_loop,
                        elapsed_ms = retry_start.elapsed().as_millis() as u64,
                        "reconnected to exchange"
                    );
                    return Ok(conn);
                },
                Err(ServerError::ExchangeConnection(ExchangeConnectionError::MaxRetries(
                    exchange,
                ))) => {
                    // Return the original error if we've exhausted retries
                    log_task!(
                        Task::ExchangeConnection,
                        Outcome::Failed,
                        subject = %pair_info.to_topic(),
                        exchange = %exchange,
                        "exhausted retries"
                    );
                    return Err(prev_err);
                },
                Err(e) => {
                    log_task!(
                        Task::ExchangeConnection,
                        Outcome::Retrying,
                        subject = %pair_info.to_topic(),
                        exchange = %exchange,
                        error = %e,
                        "failed to reconnect"
                    );
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
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        log_task!(
            Task::ExchangeConnection,
            Outcome::Retrying,
            subject = %pair_info.to_topic(),
            "retrying connection"
        );

        // Increment the retry count and filter out old requests
        let now = Instant::now();
        retry_timestamps.retain(|ts| now.duration_since(*ts) < MAX_CONN_RETRY_WINDOW);

        // Add the current timestamp to the set of retries
        retry_timestamps.push(now);
        if retry_timestamps.len() >= MAX_CONN_RETRIES {
            return Err(ServerError::ExchangeConnection(ExchangeConnectionError::MaxRetries(
                pair_info.exchange,
            )));
        }

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

    /// Get the current price for a given pair via `.borrow()` on the watch
    /// receiver, avoiding the `WatchStream` wrapper used by the websocket path.
    pub async fn get_current_price(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<Price, ServerError> {
        let (normalized_pair_info, requires_conversion) =
            self.normalize_pair_info(pair_info.clone())?;

        let price_rx =
            self.get_or_create_price_receiver(normalized_pair_info.clone(), config.clone()).await?;
        let price = *price_rx.borrow();
        if price == 0.0 {
            return Err(ServerError::PriceStreamClosed);
        }

        if requires_conversion {
            let conversion_rx = self.quote_conversion_stream(normalized_pair_info, config).await?;
            let conversion_price = *conversion_rx.borrow();
            if conversion_price == 0.0 {
                return Err(ServerError::PriceStreamClosed);
            }
            Ok(price / conversion_price)
        } else {
            Ok(price)
        }
    }

    /// Get a price receiver for the given pair or create a new stream
    async fn get_or_create_price_receiver(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceReceiver, ServerError> {
        let price_streams = self.price_streams.read().await;
        if let Some((stream_rx, _)) = price_streams.get(&pair_info).cloned() {
            return Ok(stream_rx);
        }

        drop(price_streams);

        self.init_price_stream(pair_info, config).await
    }
}
