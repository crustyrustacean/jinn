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

#[cfg(any(test, feature = "test-harness"))]
pub mod test_harness;

#[cfg(test)]
mod fabric_roundtrip_tests;
