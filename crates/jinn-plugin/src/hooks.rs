//! Hook name constants - the single source of truth for all extension points.
//!
//! Every event name that plugins can subscribe to via `ps.sub` or `ps.hook`
//! is defined here as a `pub const`. Grepping this file shows every extension
//! point in the system.
//!
//! Hook names use the `namespace::action` convention (e.g., `"app::started"`).

// ── App lifecycle ────────────────────────────────────────────────────────

/// Fired once after all plugins are loaded and the TUI is initialized.
///
/// Ctx: [`AppStartedCtx`](crate::ctx::AppStartedCtx).
pub const APP_STARTED: &str = "app::started";

// ── Session lifecycle ────────────────────────────────────────────────────

/// Fired when a new chat session is created.
///
/// Ctx: [`SessionCreatedCtx`](crate::ctx::SessionCreatedCtx).
pub const SESSION_CREATED: &str = "session::created";
