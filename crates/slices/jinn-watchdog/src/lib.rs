//! The watchdog slice — always-on behavioral supervision of in-flight turns.
//!
//! Hosts the trouper [`ServiceActor`] watchdogs ported verbatim from the
//! dormant first-party plugins (no `enabled` gates; the `[stall_watchdog]`
//! and `[tool_call_watchdog]` sections only tune when they intervene):
//!
//! - [`stall_watchdog_actor::StallWatchdogActor`] arms on every
//!   `SendToLlmProvider`, resets on `StreamToken`, and applies the
//!   end-reason policy on `StreamCompleted`. Silence past the configured
//!   window publishes the visible retry marker and re-dispatches the turn
//!   (`RetryStalledSession`); past the consecutive-restart budget it
//!   surrenders (surrender marker + `CancelStream`).
//! - [`tool_call_watchdog_actor::ToolCallWatchdogActor`] accumulates
//!   consecutive tool failures (`ToolExecutionCompleted`), trips at the
//!   configured count (trip marker + `CancelStream`), and recovers on a
//!   genuinely finished turn (`StreamCompleted` with `Finished`).
//!
//! Both actors publish through `Services`' bus (kernel dependency, see
//! Cargo.toml) and write no shared state.

pub mod stall_watchdog_actor;
pub mod tool_call_watchdog_actor;

use jinn_domain::Services;
use jinn_domain::common::state::State;
use jinn_slices::RenderFacts;
use jinn_slices::SliceHost;

/// Activates the slice: spawns both watchdog actors on trouper (their
/// `.subscribe` declarations are the readiness point).
///
/// The `[stall_watchdog]` / `[tool_call_watchdog]` config values are read
/// once from the `State` snapshot at activation (the term-slice
/// precedent) and injected into the actors. Nonsensical values (zero
/// window / zero budget / zero maximum) are floored by the config
/// accessors — the plugin-era parse-clamp semantics.
pub fn activate(host: &mut SliceHost<'_, RenderFacts>, state: &State, services: Services) {
    let (stall_cfg, tool_cfg) = {
        let snapshot = state.read();
        (
            snapshot.frontend.preferences.stall_watchdog.clone(),
            snapshot.frontend.preferences.tool_call_watchdog.clone(),
        )
    };

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
