//! Application message type for the processing loop.

use crate::common::bridge::BridgeClosure;

/// An application message for the core processing loop.
///
/// Carries typed message closures that the kanal bridge task
/// publishes to the kameo bus.
pub type AppMsg = BridgeClosure;
