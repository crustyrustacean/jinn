//! The shared application state, re-exported from [`jinn_app_state`].
//!
//! The definitions live in `jinn-app-state`, a crate beneath the kernel, so
//! that a slice can reach the state without depending on the kernel. This
//! module is a path alias only: the definitions are not duplicated here.
//! Importing `jinn_kernel::common::app_state::AppState` and
//! `jinn_app_state::AppState` name the same type.

pub use jinn_app_state::app_state::{AppState, SessionState, pin_sort_key};
pub use jinn_app_state::frontend_state::{FrontendState, PendingSessionCreation};
