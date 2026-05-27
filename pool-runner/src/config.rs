//! Configuration types and loader for the pool-runner service

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

/// A managed MM pool configuration entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPool {
    /// Human-readable name for this pool (also the pool name on the relayer)
    pub name: String,
    /// Base tickers this pool handles (e.g. ["ETH", "BTC"])
    pub base_tickers: Vec<String>,
    /// Minimum order value in USD for this pool to handle
    pub min_value_usd: f64,
    /// Optional maximum order value in USD; no upper bound if absent
    pub max_value_usd: Option<f64>,
}

/// Top-level runner configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub managed_pools: Vec<ManagedPool>,
}

/// Load the runner config from a local file path or an `s3://bucket/key` URI.
pub async fn load_runner_config(path: &str) -> Result<RunnerConfig> {
    let json = if let Some(s3_path) = path.strip_prefix("s3://") {
        load_from_s3(s3_path).await?
    } else {
        tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read config from {path}"))?
    };

    serde_json::from_str(&json).context("Failed to parse runner config JSON")
}

/// Load config bytes from an S3 URI of the form `bucket/key`
async fn load_from_s3(s3_path: &str) -> Result<String> {
    let (bucket, key) = s3_path
        .split_once('/')
        .ok_or_else(|| anyhow!("Invalid S3 path (expected bucket/key): {s3_path}"))?;

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_s3::Client::new(&config);

    let response = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .with_context(|| format!("Failed to fetch s3://{s3_path}"))?;

    let bytes =
        response.body.collect().await.context("Failed to collect S3 response body")?.into_bytes();

    String::from_utf8(bytes.to_vec()).context("S3 config is not valid UTF-8")
}
