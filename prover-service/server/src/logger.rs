//! Structured log envelope.
//!
//! Every prover-service log line should go through [`log_task!`] so it follows
//! the same `[<task>] [<outcome>] <description>` shape used by the gardener,
//! funds-manager, auth-server, and quoters. The envelope makes raw log output
//! easy to read at a glance and gives Datadog stable `@task` / `@outcome`
//! attributes for aggregation without text matching.
//!
//! ```text
//! [<task>] [<outcome>] <description>  (+ task, outcome, and any extra fields)
//! ```
//!
//! - **Task** is a closed enum of operations the prover-service performs. New
//!   tasks must be added to [`Task`] before use; the closed vocabulary is what
//!   makes `@task:X` aggregations reliable.
//! - **Outcome** is closed too: `started | ok | skipped | partial | retrying |
//!   failed`. The outcome picks the underlying tracing level (info / warn /
//!   error) so callers do not need to think about it.
//! - **Description** is the human-readable detail. Any number of structured
//!   fields can be passed before the description as `key = value`. Reserve the
//!   field name `subject` for naming WHICH thing the log line is about (here,
//!   the circuit name or listen address) so dashboards can aggregate across
//!   call sites that share a task.
//!
//! This complements the per-request `#[instrument]` spans in `prover.rs` /
//! `middleware.rs`: spans give request-level tracing context, `log_task!` gives
//! structured, aggregable events.

use tracing::Level;

/// Closed vocabulary of operations the prover-service performs.
///
/// Add a variant here before introducing a new task at a call site. The closed
/// vocabulary is what makes `@task:X` Datadog aggregations and `[task]`-prefixed
/// greps reliable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Process-level lifecycle: boot, listen, shutdown.
    ServiceLifecycle,
    /// Preprocessing of circuit proving / verification keys and layouts at boot.
    CircuitSetup,
    /// Proof generation for a single circuit (subject = circuit name). Only
    /// logged today behind the `log-invalid-bundles` feature, so allow the
    /// variant to be unused when that feature is off.
    #[cfg_attr(not(feature = "log-invalid-bundles"), allow(dead_code))]
    Prove,
    /// The warp rejection handler — final fall-through for request errors.
    HandleRejection,
}

impl Task {
    /// Stable, kebab-cased string form of this task. Used both in the `[task]`
    /// text envelope and in the `task` structured field.
    pub fn as_str(self) -> &'static str {
        match self {
            Task::ServiceLifecycle => "service-lifecycle",
            Task::CircuitSetup => "circuit-setup",
            Task::Prove => "prove",
            Task::HandleRejection => "handle-rejection",
        }
    }
}

/// Closed vocabulary of operation outcomes. Mirrors gardener's and
/// funds-manager's set so dashboards aggregating across services see the same
/// labels.
///
/// Semantics:
/// - `Started`: work has begun. Pair with a later `Ok` / `Failed` / `Skipped`.
/// - `Ok`: completed successfully.
/// - `Skipped`: nothing to do this cycle; not a failure.
/// - `Partial`: completed with a known degradation.
/// - `Retrying`: intra-call retry attempt. A failure that propagates back to
///   the caller for a later retry is `Failed`, not `Retrying`.
/// - `Failed`: errored out; the operation did not complete.
// Some variants below are part of the shared taxonomy with the other services
// but have no call site in the prover-service today. Allow them to live unused
// so the service keeps speaking the same vocabulary when future tasks need
// those outcomes.
#[allow(dead_code)]
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

    /// Map this outcome to a tracing `Level`. The choice matches the other
    /// services: successes / skips at INFO, partial / retrying at WARN, failed
    /// at ERROR. Call sites do not pick the level themselves — picking the right
    /// `Outcome` is enough.
    pub fn level(self) -> Level {
        match self {
            Outcome::Started | Outcome::Ok | Outcome::Skipped => Level::INFO,
            Outcome::Partial | Outcome::Retrying => Level::WARN,
            Outcome::Failed => Level::ERROR,
        }
    }
}

/// Emit a structured log line in the prover-service taxonomy:
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
/// The format literal follows `tracing::info!` / `println!` conventions. Any
/// number of `key = value` pairs can be passed before the literal as structured
/// fields. Use the field name `subject` to name WHICH thing the log line is
/// about so dashboards can aggregate across tasks.
///
/// The macro picks the underlying tracing level from `Outcome::level()`, so call
/// sites do not need to choose between `info!` / `warn!` / `error!` manually.
#[macro_export]
macro_rules! log_task {
    ($task:expr, $outcome:expr, $($rest:tt)+) => {
        $crate::__log_task_inner!(@munch [] $task, $outcome, $($rest)+)
    };
}

/// Implementation detail of [`log_task!`]. The tt-muncher peels off
/// `ident = expr,` field pairs one at a time before falling through to the
/// format-args terminal arm. The `=` after the identifier disambiguates "field"
/// from "first token of format args" — without it, macro_rules cannot tell the
/// two cases apart and rejects the call with `local ambiguity`.
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
