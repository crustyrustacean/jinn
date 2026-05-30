//! Per-hook context structs — the sole data contract with plugins.
//!
//! Each extension point has a dedicated Ctx struct carrying only the data
//! available at that call site. Plugins receive this data as the payload
//! argument in their `ps.sub` or `ps.hook` callbacks.
//!
//! # Design rules
//!
//! - Ctx structs are built from data the call site **already holds** — no
//!   new locks acquired, no AppState cloning.
//! - All Ctx structs implement [`serde::Serialize`] so they can be converted
//!   to Lua tables via the JSON→Lua bridge.
//! - If a plugin needs data not in the Ctx, the answer is "extend the Ctx",
//!   not "add a query".

use serde::Serialize;

// ── App lifecycle ────────────────────��───────────────────────────────────

/// Context for the [`APP_STARTED`](crate::hooks::APP_STARTED) hook.
///
/// Carries basic app metadata available at startup.
#[derive(Debug, Clone, Serialize)]
pub struct AppStartedCtx {
    /// The active session ID at startup.
    pub session_id: String,
}

// ── Session lifecycle ────────────────────────────────────────────────────

/// Context for the [`SESSION_CREATED`](crate::hooks::SESSION_CREATED) hook.
///
/// Carries the ID of the newly created session.
#[derive(Debug, Clone, Serialize)]
pub struct SessionCreatedCtx {
    /// The ID of the new session.
    pub session_id: String,
}
