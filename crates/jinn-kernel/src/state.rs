//! The kernel's shared state types.
//!
//! [`AppState`](crate::common::app_state::AppState) is composed of a session
//! map and a [`FrontendState`](jinn_app_state::FrontendState). Both are defined
//! in `jinn-app-state`, which sits beneath the kernel so that reaching the
//! state never requires depending on the kernel itself. This module re-exports
//! them at the paths the kernel's own callers already use.

pub mod frontend_state {
    pub use jinn_app_state::frontend_state::{
        FrontendCaches, FrontendState, PendingSessionCreation,
    };
}

pub use jinn_app_state::{FrontendCaches, FrontendState, PendingSessionCreation};
