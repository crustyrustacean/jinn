//! jinn-core: application runtime for the jinn agent harness.
//!
//! [`AppCore`] owns the shared application state and the kanal bridge.
//! The kanal bridge task drains the channel and publishes closures to the bus.
//! Shared state ([`State`]) is accessible from any thread via read/write guards.

pub mod app_core;

// Re-export primary types owned by this crate
pub use app_core::{AppCore, SHUTDOWN_TIMEOUT, STARTUP_TIMEOUT, wait_for_system_ready};
// Re-export State from jinn-component
pub use crate::{State, StateReadGuard, StateWriteGuard};
