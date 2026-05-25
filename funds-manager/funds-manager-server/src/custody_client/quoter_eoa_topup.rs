//! Boot-time gas top-up for v2 per-quoter EOAs.
//!
//! Each v2 quoter (`quoters` crate) operates an Ethereum EOA whose private
//! key is `keccak256(master_bot_seed || "<ticker>-quoter-<index>")`. That EOA
//! signs the `transfer_erc20` legs of the rebalance — withdrawals from the
//! quoter to the funds-manager hot wallet. The quoters server only refills
//! these EOAs in its own `setup_quoter` path, which runs at the quoters
//! process boot and nowhere else. Between quoter restarts the EOAs drain
//! through normal rebalance activity, and once a wallet falls below the
//! gas threshold its `transfer_erc20` is rejected at the RPC pre-flight
//! check with `insufficient funds for gas * price + value`. That failure
//! is logged but not propagated, so the rebalance reports "completed"
//! while the position stays stuck.
//!
//! 2026-05-25 WBTC-quoter-0 was sitting at 1.7e-6 ETH with `$149k` of WBTC
//! inventory and `$168` USDC, unable to rebalance because of this exact
//! gas-starvation. The structural fix lives in the quoters crate
//! (`31986a2 add inline gas funding check` — currently rolled back). Until
//! that lands, the funds-manager doing the top-up on its own boot is the
//! next-best safety net: every funds-manager redeploy clears the drained
//! state.
//!
//! Derivation matches `quoters/src/server/quoter_context.rs:derive_quoter_private_key`
//! exactly so addresses agree between this top-up and the quoter process
//! using the same EOA. The keccak input is `seed_bytes || quoter_key.as_bytes()`
//! where `seed_bytes = hex::decode(master_bot_seed)`.

use std::sync::Arc;

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::keccak256;
use serde::Deserialize;

use crate::custody_client::CustodyClient;
use crate::error::FundsManagerError;
use crate::helpers::{fetch_s3_object, get_secret, get_secret_prefix};
use crate::log_task;
use crate::logger::{Outcome, Task};

// -------------
// | Constants |
// -------------

/// The S3 key under which the v2 quoter config JSON lives in each chain's
/// `<chain-display>-v2-quoter-config` bucket. Matches the convention used by
/// the quoters service.
const QUOTER_CONFIG_OBJECT_KEY: &str = "quoter_config.json";

/// Suffix appended to the chain's secret prefix to resolve the v2
/// master bot seed: e.g. for `Chain::ArbitrumOne` the full secret name is
/// `/arbitrum/one/v2/master-bot-seed`. Same naming used by the quoters
/// and bot-server task definitions in `renegade-infra/modules/quoters`.
const MASTER_BOT_SEED_SECRET_SUFFIX: &str = "/v2/master-bot-seed";

/// Target ETH balance for each per-quoter EOA. Larger than the quoters
/// service's own `gas_topup_eth = 0.0001 ETH` default so that a single
/// funds-manager boot keeps every EOA funded across more than one rebalance
/// week. `top_up_gas` uses the default 10% tolerance, so it only refills
/// when balance falls below `target * 0.1 = 0.0001 ETH`.
const PER_QUOTER_EOA_TARGET_ETH: f64 = 0.001;

// ---------
// | Types |
// ---------

/// Minimal view of the v2 quoter config — only the fields needed to derive
/// each quoter's EOA address. `serde(deny_unknown_fields)` is intentionally
/// omitted so config schema additions in the quoters crate don't break this
/// top-up path.
#[derive(Debug, Deserialize)]
struct QuoterConfigDocument {
    quoters: Vec<QuoterEntry>,
}

#[derive(Debug, Deserialize)]
struct QuoterEntry {
    base_ticker: String,
    index: usize,
}

impl QuoterEntry {
    /// `{ticker}-quoter-{index}`. Must match the format produced by
    /// `QuoterConfig::quoter_key` in `quoters/src/quoter_config.rs`.
    fn quoter_key(&self) -> String {
        format!("{}-quoter-{}", self.base_ticker, self.index)
    }
}

// ----------
// | Method |
// ----------

impl CustodyClient {
    /// Derive each v2 quoter EOA for this client's chain and ensure each
    /// has gas. Best-effort: per-EOA failures are logged and the loop
    /// continues. Missing seed / missing quoter-config bucket logs a
    /// `[gas-wallet] [skipped]` and returns `Ok(())` — chains that don't
    /// host v2 quoters shouldn't fail funds-manager boot.
    pub async fn top_up_v2_quoter_eoas(self: &Arc<Self>) -> Result<(), FundsManagerError> {
        let chain = self.chain;
        log_task!(
            Task::GasWallet,
            Outcome::Started,
            chain = %chain,
            "v2 quoter EOA top-up sweep starting"
        );

        // Resolve the per-chain secret + S3 bucket. Both are derived from
        // the chain enum so this method works the same for every chain the
        // funds-manager has a custody client for.
        let secret_name = match get_secret_prefix(chain) {
            Ok(prefix) => format!("{prefix}{MASTER_BOT_SEED_SECRET_SUFFIX}"),
            Err(e) => {
                log_task!(
                    Task::GasWallet,
                    Outcome::Skipped,
                    chain = %chain,
                    error = %e,
                    "no secret prefix for chain; skipping quoter EOA top-up"
                );
                return Ok(());
            },
        };
        let bucket = format!("{chain}-v2-quoter-config");

        // Load the seed. If the secret doesn't exist on this chain (e.g. a
        // chain that's v1-only), that's a normal "no v2 quoters here" state.
        let master_bot_seed_hex = match get_secret(&secret_name, &self.aws_config).await {
            Ok(hex) => hex,
            Err(e) => {
                log_task!(
                    Task::GasWallet,
                    Outcome::Skipped,
                    chain = %chain,
                    secret = %secret_name,
                    error = %e,
                    "master bot seed not available; skipping quoter EOA top-up"
                );
                return Ok(());
            },
        };

        let seed_bytes = match hex::decode(master_bot_seed_hex.trim_start_matches("0x")) {
            Ok(b) => b,
            Err(e) => {
                log_task!(
                    Task::GasWallet,
                    Outcome::Failed,
                    chain = %chain,
                    error = %e,
                    "master bot seed not valid hex; skipping quoter EOA top-up"
                );
                return Ok(());
            },
        };

        // Load the quoter config. Same logic for missing buckets — a chain
        // without v2 quoters won't have one.
        let config_text =
            match fetch_s3_object(&bucket, QUOTER_CONFIG_OBJECT_KEY, &self.aws_config).await {
                Ok(text) => text,
                Err(e) => {
                    log_task!(
                        Task::GasWallet,
                        Outcome::Skipped,
                        chain = %chain,
                        bucket = %bucket,
                        error = %e,
                        "quoter config not available; skipping quoter EOA top-up"
                    );
                    return Ok(());
                },
            };

        let document: QuoterConfigDocument = match serde_json::from_str(&config_text) {
            Ok(d) => d,
            Err(e) => {
                log_task!(
                    Task::GasWallet,
                    Outcome::Failed,
                    chain = %chain,
                    error = %e,
                    "quoter config JSON did not parse; skipping quoter EOA top-up"
                );
                return Ok(());
            },
        };

        let total = document.quoters.len();
        let mut topped_up = 0usize;
        let mut failed = 0usize;

        for quoter in &document.quoters {
            let quoter_key = quoter.quoter_key();
            let addr = match derive_quoter_eoa(&seed_bytes, &quoter_key) {
                Ok(a) => a,
                Err(e) => {
                    failed += 1;
                    log_task!(
                        Task::GasWallet,
                        Outcome::Failed,
                        chain = %chain,
                        subject = %quoter_key,
                        error = %e,
                        "failed to derive EOA for quoter; skipping"
                    );
                    continue;
                },
            };

            // `top_up_gas` logs its own `[gas-wallet] [skipped]` line when
            // the balance is already within tolerance, and transfers ether
            // otherwise. We surface success/failure here at the per-quoter
            // granularity so the operator can see exactly which EOAs got
            // refilled without joining across the gas-wallet stream.
            match self.top_up_gas(&addr, PER_QUOTER_EOA_TARGET_ETH).await {
                Ok(()) => {
                    topped_up += 1;
                    log_task!(
                        Task::GasWallet,
                        Outcome::Ok,
                        chain = %chain,
                        subject = %quoter_key,
                        address = %addr,
                        target = PER_QUOTER_EOA_TARGET_ETH,
                        "ensured quoter EOA gas"
                    );
                },
                Err(e) => {
                    failed += 1;
                    log_task!(
                        Task::GasWallet,
                        Outcome::Failed,
                        chain = %chain,
                        subject = %quoter_key,
                        address = %addr,
                        error = %e,
                        "failed to ensure quoter EOA gas; continuing"
                    );
                },
            }
        }

        // `topped_up` collapses "actually refilled" and "balance was already
        // fine into the same success bucket — operators care about the
        // failure count, not which of the two each EOA hit. The precise
        // refill-vs-skip split is available in the per-EOA `[gas-wallet]`
        // lines that `top_up_gas` emits internally.
        let skipped = total.saturating_sub(topped_up + failed);

        log_task!(
            Task::GasWallet,
            Outcome::Ok,
            chain = %chain,
            total = total,
            topped_up = topped_up,
            skipped = skipped,
            failed = failed,
            "v2 quoter EOA top-up sweep complete"
        );

        Ok(())
    }
}

// ---------------
// | Derivation  |
// ---------------

/// Derive a v2 quoter EOA from the master bot seed bytes and the quoter
/// key. Mirrors `quoters/src/server/quoter_context.rs:derive_quoter_private_key`:
/// `pkey = keccak256(seed_bytes || quoter_key)`, EOA = secp256k1 address of
/// the resulting private key.
///
/// Returns the address formatted as a 0x-prefixed lowercase hex string so
/// it slots into `CustodyClient::top_up_gas` (which downstream parses with
/// `Address::from_str`) and into log fields without further conversion.
fn derive_quoter_eoa(seed_bytes: &[u8], quoter_key: &str) -> Result<String, FundsManagerError> {
    let mut input = Vec::with_capacity(seed_bytes.len() + quoter_key.len());
    input.extend_from_slice(seed_bytes);
    input.extend_from_slice(quoter_key.as_bytes());

    let hash = keccak256(&input);
    let signer = PrivateKeySigner::from_slice(hash.as_slice())
        .map_err(|e| FundsManagerError::custom(format!("PrivateKeySigner::from_slice: {e}")))?;
    Ok(format!("{:#x}", signer.address()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity check: derivation is deterministic. A small fixed seed and
    /// quoter key always produce the same address. If this ever diverges,
    /// the derivation in this module is out of sync with
    /// `quoters/src/server/quoter_context.rs`.
    #[test]
    fn derivation_is_deterministic() {
        let seed = hex::decode("00112233445566778899aabbccddeeff").unwrap();
        let a = derive_quoter_eoa(&seed, "ARB-quoter-0").unwrap();
        let b = derive_quoter_eoa(&seed, "ARB-quoter-0").unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("0x"));
        assert_eq!(a.len(), 42);
    }

    #[test]
    fn quoter_key_format_matches_quoters_crate() {
        let q = QuoterEntry { base_ticker: "ARB".to_string(), index: 0 };
        assert_eq!(q.quoter_key(), "ARB-quoter-0");
    }
}
