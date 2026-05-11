//! Result type returned by intent handlers.
//!
//! Carries commands to be dispatched to the actor system.
//! Lives in the protocol crate so that slice crates can return it
//! without depending on `nullslop-intent`.

use crate::Command;

/// What an intent handler returns after processing an intent.
#[derive(Debug)]
pub struct IntentResult {
    /// Commands to send to the actor system.
    pub commands: Vec<Command>,
}

impl IntentResult {
    /// An empty result with no commands.
    #[must_use]
    pub fn empty() -> Self {
        Self { commands: vec![] }
    }

    /// A result with commands.
    #[must_use]
    pub fn with_commands(commands: Vec<Command>) -> Self {
        Self { commands }
    }
}
