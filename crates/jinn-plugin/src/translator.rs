//! Translator callback type for converting plugin command names to typed Commands.
//!
//! The translator is injected at construction time so the plugin crate
//! remains agnostic about which domain commands exist. The wiring layer
//! provides the concrete mapping.

use std::sync::Arc;

/// A function that maps a command name and JSON payload to a typed Command.
///
/// Returns `Some(Command)` if the name is recognized and translation succeeds,
/// or `None` if the name is unknown.
pub type TranslatorFn =
    Arc<dyn Fn(&str, serde_json::Value) -> Option<jinn_domain::Command> + Send + Sync>;

/// A no-op translator for test contexts where plugins are not needed.
pub fn noop_translator() -> TranslatorFn {
    Arc::new(|name, _payload| {
        tracing::warn!(name, "no translator configured, command dropped");
        None
    })
}
