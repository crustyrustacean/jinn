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

// Re-export primary types owned by this crate
pub use actor_sink::ActorMessageSink;
pub use app_core::{spawn_forwarding_task, AppCore, SHUTDOWN_TIMEOUT, TickResult};
pub use app_msg::AppMsg;
// Re-export State from nullslop-component
pub use nullslop_component::{State, StateReadGuard, StateWriteGuard};
