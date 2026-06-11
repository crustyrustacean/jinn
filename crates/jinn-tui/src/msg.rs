//! Message channel for the TUI event loop.
//!
//! Provides a unified message type that merges crossterm terminal events,
//! periodic tick messages, and commands into a single stream.


pub mod handler;
pub mod sender;

pub use sender::MsgSender;

/// A unified message from any source.
///
/// Merges crossterm terminal events, periodic tick messages,
/// and commands (from key handling or actors) into a single stream
/// consumed by the main event loop.
pub enum Msg {
    /// Periodic tick for render refresh.
    Tick,
    /// A crossterm terminal event (key press, resize, etc.).
    Input(crossterm::event::Event),
    /// A bus closure from key handling or external injection.
    Bridge(jinn_domain::BridgeClosure),
}

impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tick => f.write_str("Tick"),
            Self::Input(e) => f.debug_tuple("Input").field(e).finish(),
            Self::Bridge(_) => f.write_str("Bridge(<closure>)"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code, panics are acceptable"
    )]

    use super::*;


    //FIXME: disabled during actor migration — rewrite test for Msg::Bridge
    // #[rstest::rstest]
    // fn bridge_message_carries_closure() { ...
}
