//! The watchdog slice — always-on behavioral supervision of in-flight turns.
//!
//! Hosts the trouper [`ServiceActor`] watchdogs ported verbatim from the
//! dormant first-party plugins (no `enabled` gates; the `[stall_watchdog]`
//! and `[tool_call_watchdog]` sections only tune when they intervene):
//!
//! - [`stall_watchdog_actor::StallWatchdogActor`] arms on every
//!   `SendToLlmProvider`, resets the silence clock on every `StreamActivity`,
//!   and applies the end-reason policy on `StreamCompleted` — which also
//!   decides when the restart budget clears (a completed generation does).
//!   Silence past the configured window publishes the visible retry marker
//!   and re-dispatches the turn (`RetryStalledSession`); past the budget of
//!   silent stalls between completed generations it surrenders (surrender
//!   marker + `CancelStream`).
//! - [`tool_call_watchdog_actor::ToolCallWatchdogActor`] accumulates
//!   consecutive tool failures (`ToolExecutionCompleted`), trips at the
//!   configured count (trip marker + `CancelStream`), and recovers on a
//!   genuinely finished turn (`StreamCompleted` with `Finished`).
//!
//! Both actors publish through `Services`' bus (kernel dependency, see
//! Cargo.toml) and write no shared state.

pub mod stall_watchdog_actor;
pub mod tool_call_watchdog_actor;

use jinn_kernel::Services;
use jinn_kernel::common::state::State;
use jinn_preferences_config::schemas::StallWatchdogConfig;
use jinn_preferences_config::schemas::ToolCallWatchdogConfig;
use jinn_slices::RenderFacts;
use jinn_slices::SliceHost;

/// Activates the slice: spawns both watchdog actors on trouper (their
/// `.subscribe` declarations are the readiness point).
///
/// The `[watchdog.stall]` / `[watchdog.tool_call]` config values are read
/// from the configuration layer at activation (the term-slice precedent)
/// and injected into the actors. Nonsensical values (zero window / zero
/// budget / zero maximum) are floored by the config accessors — the
/// plugin-era parse-clamp semantics.
pub fn activate(host: &mut SliceHost<'_, RenderFacts>, _state: &State, services: Services) {
    let stall_cfg = services
        .config
        .get::<StallWatchdogConfig>()
        .unwrap_or_default();
    let tool_cfg = services
        .config
        .get::<ToolCallWatchdogConfig>()
        .unwrap_or_default();

    // The stall watchdog needs the system for its self-addressed tick;
    // the config floors make a zero window or budget behave like the
    // plugin-era parse clamps (≥ 1).
    let stall_timeout_secs = stall_cfg.effective_timeout_secs();
    let stall_max_restarts = stall_cfg.effective_max_restarts();
    stall_watchdog_actor::StallWatchdogActor::spawn(
        host.system(),
        stall_watchdog_actor::StallWatchdogActorDeps {
            services: services.clone(),
            timeout_ms: stall_timeout_secs.saturating_mul(1_000),
            max_restarts: stall_max_restarts,
            tick_interval: stall_watchdog_actor::STALL_TICK_INTERVAL,
        },
    );

    tool_call_watchdog_actor::ToolCallWatchdogActor::spawn(
        host.system(),
        tool_call_watchdog_actor::ToolCallWatchdogActorDeps {
            services,
            max_failures: tool_cfg.effective_max_failures(),
        },
    );
}
