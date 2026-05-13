//! nullslop-core: application runtime for the nullslop agent harness.
//!
//! The processing pipeline ([`AppCore`]) receives messages and forwards them
//! to the actor host. Shared state ([`State`]) is accessible from any thread
//! via read/write guards.
//!
//! Phase 7: The bus has been deleted. An async forwarding task continuously
//! drains the `AppMsg` channel and forwards directly to the actor host.
//! The main loop is input + rendering only.

pub mod actor_sink;
pub mod app_core;
pub mod app_msg;
pub mod core_notification;

// Re-export primary types owned by this crate
pub use actor_sink::ActorMessageSink;
pub use app_core::{
    AppCore, SHUTDOWN_TIMEOUT, STARTUP_TIMEOUT, coordinated_shutdown, spawn_forwarding_task,
    wait_for_system_ready,
};
pub use app_msg::AppMsg;
pub use core_notification::CoreNotification;
// Re-export State from nullslop-component
pub use crate::{State, StateReadGuard, StateWriteGuard};
