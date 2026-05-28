//! Structured log envelope.
//!
//! Every funds-manager log line should go through [`log_task!`] so it
//! follows the same `[<task>] [<outcome>] <description>` shape used by
//! gardener. This makes the funds-manager activity easy to read in raw log
//! output and aggregable in Datadog via the `@task` / `@outcome` JSON
//! fields the macro attaches.
//!
//! Pattern (mirrors `gardener/src/utils/logger.ts`):
//!
//! ```text
//! [<task>] [<outcome>] <description>  (+ structured fields)
//! ```
//!
//! - **Task** is a closed enum of operations funds-manager performs. New tasks
//!   must be added to [`Task`] before use; the closed vocabulary is what makes
//!   `@task:X` aggregations reliable.
//! - **Outcome** is closed too: `started | ok | skipped | partial | retrying |
//!   failed`. The outcome picks the underlying tracing level (info/warn/error)
//!   so callers do not need to think about it.
//! - **Description** is the human-readable detail. Any number of structured
//!   fields can be passed before the description as `key = value`. Reserve the
//!   field name `subject` for naming WHICH thing the log line is about (vault,
//!   address, ticker, route) — that keeps dashboards aggregable across call
//!   sites that share a task.
//!
//! Usage:
//!
//! ```ignore
//! use crate::logger::{Outcome, Task};
//! use crate::log_task;
//!
//! log_task!(Task::ServiceLifecycle, Outcome::Started, "boot beginning");
//!
//! log_task!(
//!     Task::SignRpc,
//!     Outcome::Failed,
//!     subject = "eth_signTypedData_v4",
//!     error = %e,
//!     "create_transaction failed after limiter cooldown"
//! );
//! ```

use tracing::Level;

/// Closed vocabulary of operations funds-manager performs.
///
/// Add a variant here before introducing a new task at a call site. The
/// closed vocabulary is what makes `@task:X` Datadog aggregations and
/// `[task]`-prefixed greps reliable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Process-level lifecycle: boot, listen, shutdown, panic.
    ServiceLifecycle,
    /// Top-level handling of an HTTP/warp request before dispatch.
    HandleHttpRequest,
    /// HMAC authentication of incoming requests.
    Auth,
    /// Handling of an incoming JSON-RPC request (RPC shim).
    HandleRpcRequest,
    /// Fireblocks EIP-712 typed-data signing (`eth_signTypedData_v4`).
    SignRpc,
    /// Polling a Fireblocks transaction by ID until terminal state.
    PollFireblocksTx,
    /// The process-wide Fireblocks rate limiter (cooldown gate, etc.).
    FireblocksRateLimit,
    /// Inbound Fireblocks webhook delivery (signature verification, dispatch).
    FireblocksWebhook,
    /// Reading vault balances from Fireblocks.
    FetchVaultBalances,
    /// Reading vault account / asset metadata from Fireblocks.
    FetchFireblocksMetadata,
    /// Custody transfers between vaults and hot wallets.
    CustodyTransfer,
    /// Withdrawal of custody funds (vault → hot wallet, hot wallet → external).
    Withdraw,
    /// Deposit-address resolution and ERC20 deposit handling.
    Deposit,
    /// Gas wallet creation / refill / status transitions.
    GasWallet,
    /// Refilling the gas sponsor contract.
    GasSponsorRefill,
    /// Hot wallet creation and management.
    HotWallet,
    /// ERC20 allowance approvals.
    Erc20Approve,
    /// Fee indexing from the Renegade darkpool.
    IndexFees,
    /// Fee redemption flow.
    RedeemFees,
    /// Inventory swap flows (swap_immediate, swap_to_target).
    Swap,
    /// Fetching execution-venue quotes (bebop, lifi, okx, cowswap).
    FetchQuote,
    /// Submitting orders / placing swap legs at an execution venue.
    SubmitOrder,
    /// On-chain transactions (sending, retrying, awaiting confirmations).
    OnChainTx,
    /// Fetching wallets / metadata from the Renegade relayer.
    FetchRelayerWallet,
    /// Database read/write operations.
    Db,
    /// Recording metrics (cost, NAV, etc.) to the metrics backend.
    RecordMetric,
    /// Errors surfaced via the warp rejection handler.
    HandleRejection,
    /// Untagged failures that escape to the panic hook.
    UncaughtPanic,
}

impl Task {
    /// Stable, kebab-cased string form of this task. Used both in the
    /// `[task]` text envelope and in the `task` structured field.
    pub fn as_str(self) -> &'static str {
        match self {
            Task::ServiceLifecycle => "service-lifecycle",
            Task::HandleHttpRequest => "handle-http-request",
            Task::Auth => "auth",
            Task::HandleRpcRequest => "handle-rpc-request",
            Task::SignRpc => "sign-rpc",
            Task::PollFireblocksTx => "poll-fireblocks-tx",
            Task::FireblocksRateLimit => "fireblocks-rate-limit",
            Task::FireblocksWebhook => "fireblocks-webhook",
            Task::FetchVaultBalances => "fetch-vault-balances",
            Task::FetchFireblocksMetadata => "fetch-fireblocks-metadata",
            Task::CustodyTransfer => "custody-transfer",
            Task::Withdraw => "withdraw",
            Task::Deposit => "deposit",
            Task::GasWallet => "gas-wallet",
            Task::GasSponsorRefill => "gas-sponsor-refill",
            Task::HotWallet => "hot-wallet",
            Task::Erc20Approve => "erc20-approve",
            Task::IndexFees => "index-fees",
            Task::RedeemFees => "redeem-fees",
            Task::Swap => "swap",
            Task::FetchQuote => "fetch-quote",
            Task::SubmitOrder => "submit-order",
            Task::OnChainTx => "on-chain-tx",
            Task::FetchRelayerWallet => "fetch-relayer-wallet",
            Task::Db => "db",
            Task::RecordMetric => "record-metric",
            Task::HandleRejection => "handle-rejection",
            Task::UncaughtPanic => "uncaught-panic",
        }
    }
}

/// Closed vocabulary of operation outcomes. Mirrors gardener's set.
///
/// Semantics:
/// - `Started`: work has begun. Pair with a later `Ok`/`Failed`/`Skipped`.
/// - `Ok`: completed successfully.
/// - `Skipped`: nothing to do this cycle; not a failure.
/// - `Partial`: completed with a known degradation (cache fallback, some legs
///   failed but the operation continued).
/// - `Retrying`: intra-call retry attempt. A failure that propagates back to
///   the caller for a later retry is `Failed`, not `Retrying`.
/// - `Failed`: errored out; the operation did not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Work has begun.
    Started,
    /// Completed successfully.
    Ok,
    /// Nothing to do; not a failure.
    Skipped,
    /// Completed with known degradation.
    Partial,
    /// Intra-call retry attempt.
    Retrying,
    /// Errored out; did not complete.
    Failed,
}

impl Outcome {
    /// Stable kebab-cased string form for the `[outcome]` envelope and the
    /// structured `outcome` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Started => "started",
            Outcome::Ok => "ok",
            Outcome::Skipped => "skipped",
            Outcome::Partial => "partial",
            Outcome::Retrying => "retrying",
            Outcome::Failed => "failed",
        }
    }

    /// Map this outcome to a tracing `Level`. The choice matches gardener:
    /// successes / skips at INFO, partial / retrying at WARN, failed at
    /// ERROR. Call sites do not pick the level themselves — picking the
    /// right `Outcome` is enough.
    pub fn level(self) -> Level {
        match self {
            Outcome::Started | Outcome::Ok | Outcome::Skipped => Level::INFO,
            Outcome::Partial | Outcome::Retrying => Level::WARN,
            Outcome::Failed => Level::ERROR,
        }
    }
}

/// Emit a structured log line in the funds-manager taxonomy:
///
/// ```text
/// [<task>] [<outcome>] <description>     (+ task, outcome, and any extra fields)
/// ```
///
/// Signature:
///
/// ```ignore
/// log_task!(<task>, <outcome>, [field = value, ...] <fmt literal> [, args...]);
/// ```
///
/// The format literal follows `tracing::info!` / `println!` conventions.
/// Any number of `key = value` pairs can be passed before the literal as
/// structured fields. Use the field name `subject` to name WHICH thing
/// the log line is about (vault, ticker, address) so dashboards can
/// aggregate across tasks.
///
/// The macro picks the underlying tracing level from `Outcome::level()`,
/// so call sites do not need to choose between `info!` / `warn!` /
/// `error!` manually.
#[macro_export]
macro_rules! log_task {
    ($task:expr, $outcome:expr, $($rest:tt)+) => {
        $crate::__log_task_inner!(@munch [] $task, $outcome, $($rest)+)
    };
}

/// Implementation detail of [`log_task!`]. The tt-muncher peels off
/// `ident = expr,` field pairs one at a time before falling through to
/// the format-args terminal arm. The `=` after the identifier
/// disambiguates "field" from "first token of format args" — without it,
/// macro_rules cannot tell the two cases apart and rejects the call with
/// `local ambiguity`.
#[doc(hidden)]
#[macro_export]
macro_rules! __log_task_inner {
    // Munch one field — Debug-formatted value (tracing's `?expr` shorthand)
    (@munch [$($fields:tt)*] $task:expr, $outcome:expr, $field:ident = ?$val:expr, $($rest:tt)+) => {
        $crate::__log_task_inner!(@munch [$($fields)* $field = ?$val,] $task, $outcome, $($rest)+)
    };
    // Munch one field — Display-formatted value (tracing's `%expr` shorthand)
    (@munch [$($fields:tt)*] $task:expr, $outcome:expr, $field:ident = %$val:expr, $($rest:tt)+) => {
        $crate::__log_task_inner!(@munch [$($fields)* $field = %$val,] $task, $outcome, $($rest)+)
    };
    // Munch one field — plain value
    (@munch [$($fields:tt)*] $task:expr, $outcome:expr, $field:ident = $val:expr, $($rest:tt)+) => {
        $crate::__log_task_inner!(@munch [$($fields)* $field = $val,] $task, $outcome, $($rest)+)
    };
    // Out of fields; emit the event at the level chosen by Outcome::level()
    (@munch [$($fields:tt)*] $task:expr, $outcome:expr, $($arg:tt)+) => {{
        let __task = $task;
        let __outcome = $outcome;
        let __task_str = __task.as_str();
        let __outcome_str = __outcome.as_str();
        match __outcome.level() {
            ::tracing::Level::ERROR => ::tracing::error!(
                task = __task_str,
                outcome = __outcome_str,
                $($fields)*
                "[{}] [{}] {}",
                __task_str,
                __outcome_str,
                ::std::format_args!($($arg)+)
            ),
            ::tracing::Level::WARN => ::tracing::warn!(
                task = __task_str,
                outcome = __outcome_str,
                $($fields)*
                "[{}] [{}] {}",
                __task_str,
                __outcome_str,
                ::std::format_args!($($arg)+)
            ),
            ::tracing::Level::INFO => ::tracing::info!(
                task = __task_str,
                outcome = __outcome_str,
                $($fields)*
                "[{}] [{}] {}",
                __task_str,
                __outcome_str,
                ::std::format_args!($($arg)+)
            ),
            _ => {}
        }
    }};
}

/// Install a panic hook that routes panics through [`log_task!`]. Without
/// this, a panic emits an unstructured backtrace that cannot be filtered
/// alongside other `failed` outcomes. Call once from `main` before any
/// worker spawns.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        crate::log_task!(
            Task::UncaughtPanic,
            Outcome::Failed,
            location = %location,
            payload = %payload,
            "panic at {}: {}",
            location,
            payload
        );
        default(info);
    }));
}
