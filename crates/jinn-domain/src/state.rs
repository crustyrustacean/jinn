//! The kernel's shared state types.
//!
//! [`AppState`](crate::common::app_state::AppState) is composed of a session
//! map and a [`FrontendState`]. Both live here rather than under a feature
//! module so the shared state has no dependency on any one feature's code —
//! the frontend state was previously declared under `feat/ui/`, which made a
//! 7k-line standalone chat-log component load-bearing for every consumer of
//! `AppState`.

pub mod frontend_state;

pub use frontend_state::{FrontendCaches, FrontendState};
