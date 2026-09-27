//! Marker trait for types publishable on the message fabric.
//!
//! Every command/event struct that should be routable through the
//! fabric must implement this trait. Discoverable via
//! `rg "impl BusMessage"`.

/// Marker for types publishable on the message bus.
///
/// No methods — this exists purely for discoverability and compile-time
/// bounds checking.
pub use jinn_slices::BusMessage;

// The bus test harness itself lives in `jinn-testutil`. This module adds the
// kernel-side `services()` / `actor_deps()` builders that need `Services`.
pub mod harness_services;
pub use harness_services::HarnessServices;

#[cfg(test)]
mod fabric_roundtrip_tests;
