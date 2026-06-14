//! Marker trait for types publishable on the kameo message bus.
//!
//! Every command/event struct that should be routable through the
//! [`MessageBus`](kameo_actors::message_bus::MessageBus) must implement
//! this trait. Discoverable via `rg "impl BusMessage"`.

/// Marker for types publishable on the message bus.
///
/// Implementors must be `Clone + Send + 'static` to satisfy
/// kameo's `MessageBus` requirements. No methods — this exists
/// purely for discoverability and compile-time bounds checking.
pub trait BusMessage: Clone + Send + 'static {}

#[cfg(test)]
pub mod test_harness;
