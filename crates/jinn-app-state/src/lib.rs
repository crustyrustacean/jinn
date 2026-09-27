//! The shared application state layer.
//!
//! [`AppState`] is the single source of truth for what the user sees and how
//! the application is currently behaving. It lives here, beneath the kernel,
//! so that any consumer can reach the state without taking a dependency on
//! the kernel itself — the kernel depends on this crate, never the reverse.
//!
//! This crate holds:
//!
//! - [`AppState`] and its [`FrontendState`](frontend_state::FrontendState)
//!   group struct, plus the theme-sensitive caches beside them.
//! - The session-creation operations ([`session_creation`]) that mutate
//!   `AppState` and return the messages the lifecycle actors act on.
//!
//! The states are composed of value types and slice-owned `-msg` contracts
//! only. Nothing here depends on a slice implementation, on the kernel, or on
//! the actor bus, which is what makes the state reachable from anywhere in
//! the workspace.

pub mod app_state;
#[cfg(test)]
mod app_state_tests;
pub mod frontend_state;
pub mod session_creation;
pub mod slice_action;

pub use app_state::pin_sort_key;
pub use app_state::{AppState, SessionState};
pub use frontend_state::{FrontendCaches, FrontendState, PendingSessionCreation};
